// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Remove unused verification materials and preserve the identity of retained Sources.
use super::ParsedSpec;
use rpm_spec::ast::{SpecItem, Tag};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

const SUFFIXES: &[&str] = &[".asc", ".sig", ".sign", ".pub"];

pub(crate) struct Cleanup {
    pub spec: ParsedSpec<'static>,
    pub removed: Vec<u32>,
    pub numbers: BTreeMap<u32, u32>,
}

/// Hash filling keeps old numbers until selected fields have been rendered.
pub(crate) fn remove_unused(
    parsed: &ParsedSpec<'_>,
    defines: &[String],
) -> Result<Option<ParsedSpec<'static>>, String> {
    Ok(cleanup(parsed, defines, false)?.map(|c| c.spec))
}

pub(crate) fn cleanup(
    parsed: &ParsedSpec<'_>,
    defines: &[String],
    renumber: bool,
) -> Result<Option<Cleanup>, String> {
    let resolved = match super::sources::resolve(parsed, defines) {
        Ok(r) if r.incomplete.is_none() => r,
        _ => return Ok(None),
    };
    let source = parsed.source();
    if resolved.sources.values().any(|s| {
        s.expression.contains("%{SOURCE")
            || s.expression.contains("%SOURCE")
            || s.expression.contains("%{S:")
    }) {
        return Ok(None);
    }
    let mut declarations = BTreeMap::new();
    for (number, value) in &resolved.sources {
        let Some(index) = parsed.parsed.spec.items.iter().position(|item|
            matches!(item, SpecItem::Preamble(p) if matches!(p.tag,Tag::Source(_)) && p.data.start_byte == value.span.bytes.start))
        else { return Ok(None) };
        let mut start = value.span.bytes.start;
        for previous in parsed.parsed.spec.items[..index].iter().rev() {
            let SpecItem::Comment(comment) = previous else {
                break;
            };
            let Some(raw) = source.get(comment.data.start_byte..comment.data.end_byte) else {
                return Err("invalid Source marker range".into());
            };
            if comment.data.end_byte != start
                || !(raw.trim() == "#!RemoteAsset"
                    || raw.trim_start().starts_with("#!RemoteAsset:"))
            {
                break;
            }
            start = comment.data.start_byte;
        }
        declarations.insert(*number, start..value.span.bytes.end);
    }
    // Ignore declarations when searching for consumers; unrelated Source URLs
    // must not make all detached signatures appear referenced.
    let mut body = source.to_owned();
    let mut ranges: Vec<_> = declarations.values().cloned().collect();
    ranges.sort_by_key(|r| r.start);
    for range in ranges.into_iter().rev() {
        // Mask instead of shortening: reference ranges stay bound to the input.
        body.replace_range(range.clone(), &" ".repeat(range.len()));
    }
    if !resolved.sources.contains_key(&0) {
        return Ok(None);
    }
    // These forms can select numbered sources without a literal SOURCE macro.
    // Keep them intact until their argument semantics can be mapped reliably.
    if body.contains("%{S:")
        || body.contains("%S ")
        || body.contains("SOURCES")
        || body.contains("%{lua:")
        || body.contains("%{expand:")
        || body.contains("%(")
        || body
            .lines()
            .any(|line| line.contains("_sourcedir") && line.contains('*'))
        || body.contains("%{") && body.contains("SOURCE%")
        || body.lines().any(|line| {
            (line.contains("%setup")
                || line.contains("%autosetup")
                || line.trim_start().starts_with("BuildOption(prep):"))
                && line
                    .split_whitespace()
                    .any(|word| word.starts_with("-a") || word.starts_with("-b"))
        })
    {
        return Ok(None);
    }
    let (uses, opaque) = references(&body);
    if opaque {
        return Ok(None);
    }
    let used: BTreeSet<_> = uses.iter().map(|(_, n)| *n).collect();
    let mut removed = Vec::new();
    for (&number, value) in &resolved.sources {
        if number == 0 || used.contains(&number) {
            continue;
        } // %setup uses Source0 implicitly.
        let Ok(url) = &value.url else { continue };
        let Ok(name) = super::sources::filename(url) else {
            continue;
        };
        let remote_path = url::Url::parse(url).ok().map(|url| url.path().to_owned());
        let Some(suffix) = SUFFIXES.iter().find(|suffix| {
            name.ends_with(**suffix)
                || remote_path
                    .as_ref()
                    .is_some_and(|path| path.ends_with(**suffix))
        }) else {
            continue;
        };
        let expression = value
            .expression
            .rsplit('/')
            .next()
            .unwrap_or(&value.expression);
        if body.contains(name) || body.contains(expression) || body.contains(suffix) || opaque {
            continue;
        }
        removed.push(number);
    }
    let mut numbers = BTreeMap::new();
    for (next, old) in resolved
        .sources
        .keys()
        .filter(|n| !removed.contains(n))
        .enumerate()
    {
        numbers.insert(
            *old,
            if renumber {
                u32::try_from(next).map_err(|_| "too many Sources")?
            } else {
                *old
            },
        );
    }
    if removed.is_empty() && numbers.iter().all(|(old, new)| old == new) {
        return Ok(None);
    }
    let mut changes: Vec<(Range<usize>, String)> = removed
        .iter()
        .map(|n| (declarations[n].clone(), String::new()))
        .collect();
    if renumber {
        for (range, old) in uses {
            let Some(new) = numbers.get(&old) else {
                return Ok(None);
            };
            if *new != old {
                changes.push((range, new.to_string()));
            }
        }
        for (&old, &new) in &numbers {
            let range = &resolved.sources[&old].span.bytes;
            let raw = source
                .get(range.clone())
                .ok_or("invalid Source declaration")?;
            let colon = raw.find(':').ok_or("Source declaration has no colon")?;
            if old != new {
                changes.push((range.start..range.start + colon, format!("Source{new}")));
            }
        }
    }
    changes.sort_by_key(|(r, _)| r.start);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    for (range, value) in changes {
        if range.start < cursor {
            return Err("overlapping Source cleanup ranges".into());
        }
        output.push_str(
            source
                .get(cursor..range.start)
                .ok_or("invalid Source cleanup range")?,
        );
        output.push_str(&value);
        cursor = range.end;
    }
    output.push_str(source.get(cursor..).ok_or("invalid Source cleanup end")?);
    let candidate = ParsedSpec::parse(output);
    if candidate
        .parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == rpm_spec::parse_result::Severity::Error)
    {
        return Err("Source cleanup produced an invalid SPEC".into());
    }
    let result = super::sources::resolve(&candidate, defines)?;
    if result.sources.len() != numbers.len()
        || numbers.iter().any(|(old, new)| {
            result.sources.get(new).is_none_or(|s| {
                s.url != resolved.sources[old].url || s.digest != resolved.sources[old].digest
            })
        })
    {
        return Err("Source cleanup changed retained material identity".into());
    }
    Ok(Some(Cleanup {
        spec: candidate,
        removed,
        numbers,
    }))
}

/// Only literal macro references have writable digit ranges. Bare, escaped or
/// computed SOURCE names prevent renumbering instead of being guessed.
fn references(source: &str) -> (Vec<(Range<usize>, u32)>, bool) {
    let mut result = Vec::new();
    let mut opaque = false;
    for (start, _) in source.match_indices("SOURCE") {
        let digits = start + 6;
        let end = digits
            + source[digits..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
        let prefix = &source[..start];
        let braced = ["%{", "%{?", "%{!?"].iter().any(|p| prefix.ends_with(p));
        let simple = prefix.ends_with('%') && !prefix.ends_with("%%");
        let boundary = source[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let escaped = ["%%{", "%%{?", "%%{!?"].iter().any(|p| prefix.ends_with(p));
        if digits == end || !boundary || !(braced || simple) || escaped {
            opaque = true;
            continue;
        }
        if braced && !source[end..].starts_with('}') {
            opaque = true;
            continue;
        }
        match source[digits..end].parse() {
            Ok(number) => result.push((digits..end, number)),
            Err(_) => opaque = true,
        }
    }
    (result, opaque)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_multiple_unused_materials_and_rewrites_references() {
        let input = "Name: pkg\nVersion: 1\nSource0: https://example.org/pkg.tar.gz\n#!RemoteAsset\nSource2: https://example.org/pkg.sig\n#!RemoteAsset\n#!RemoteAsset: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nSource3: https://example.org/key.asc#/%{name}.keyring\nSource5: file.xsd\nSource10: file.dtd\n%install\ncp %{SOURCE5} %{SOURCE10} .\n";
        let fixed = cleanup(&ParsedSpec::parse(input), &[], true)
            .unwrap()
            .unwrap();
        assert_eq!(fixed.removed, vec![2, 3]);
        assert_eq!(
            fixed.spec.source(),
            "Name: pkg\nVersion: 1\nSource0: https://example.org/pkg.tar.gz\nSource1: file.xsd\nSource2: file.dtd\n%install\ncp %{SOURCE1} %{SOURCE2} .\n"
        );
        assert!(cleanup(&fixed.spec, &[], true).unwrap().is_none());
    }
    #[test]
    fn keeps_consumed_material_identity_when_compacting_numbers() {
        let input = "Name: pkg\nSource0: a.tar.gz\nSource2: a.sig\n%prep\n";
        for reference in [
            "cat %{SOURCE2}",
            "# %{SOURCE2}",
            "cat a.sig",
            "gpg --verify *.sig",
        ] {
            let spec = ParsedSpec::parse(format!("{input}{reference}\n"));
            let fixed = cleanup(&spec, &[], true).unwrap().unwrap();
            assert!(fixed.removed.is_empty());
            assert_eq!(
                fixed.spec.source(),
                spec.source()
                    .replace("Source2:", "Source1:")
                    .replace("%{SOURCE2}", "%{SOURCE1}")
            );
        }
        for reference in [
            "%setup -a 2",
            "BuildOption(prep): -a 2",
            "cat %{S:2}",
            "cat %{SOURCE%{number}}",
            "cp %{_sourcedir}/* .",
            "%{lua:print(1)}",
        ] {
            assert!(
                cleanup(
                    &ParsedSpec::parse(format!("{input}{reference}\n")),
                    &[],
                    true
                )
                .unwrap()
                .is_none(),
                "{reference}"
            );
        }
    }
}
