// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Checks generated facts against the manifest and distribution defaults.
//!
//! Build expected facts from the manifest, not by asking the renderer to render
//! again: the latter would repeat rendering mistakes instead of detecting them.
//! This still shares rpm-spec with parsing; it is not native RPM validation.

use super::ParsedSpec;
use crate::profile::Profile;
use crate::render::{
    RenderError,
    manifest::{Files, Manifest, PackageBody, Source, Stage, SubpackageName, Vcs},
};
use rpm_spec::{
    ast::{
        BuildScriptKind, BuildScriptPlacement, ChangelogItem, CommentStyle, FileDirective,
        FilesContent, PackageName, PreambleContent, PreambleItem, Section, Span, SpecItem,
        SubpkgRef, Tag, TagValue, Text, TextSegment,
    },
    parser::{Input, ParserState, deps::parse_dep_expr, text::parse_text},
};

type ExpectedTag = (Tag, Option<&'static str>, TagValue);

/// Checks candidate facts against the manifest and profile.
pub(crate) fn run(
    spec: &ParsedSpec<'_>,
    manifest: &Manifest,
    profile: &Profile,
) -> Result<(), RenderError> {
    let source = spec.source;
    let parsed = &spec.parsed;
    if !parsed.diagnostics.is_empty() {
        return Err(RenderError::Invalid(format!(
            "generated SPEC produced parser diagnostics:\n{}",
            parsed
                .diagnostics
                .iter()
                .map(|d| format!("- {}: {}", d.code.as_deref().unwrap_or("parser"), d.message))
                .collect::<Vec<_>>()
                .join("\n")
        )));
    }

    let package = &manifest.package;
    let mut tags = Vec::new();
    for (tag, value) in [
        (Tag::Name, package.name.as_str()),
        (Tag::Version, &package.version),
        (Tag::Release, &profile.release),
        (Tag::License, &package.license),
        (Tag::URL, &package.url),
    ] {
        tags.push((tag, None, TagValue::Text(text(value)?)));
    }
    if manifest.package.noarch {
        tags.push((
            Tag::BuildArch,
            None,
            TagValue::ArchList(vec![text("noarch")?]),
        ));
    }
    if let Some(system) = &manifest.build.system {
        tags.push((
            Tag::Other("BuildSystem".into()),
            None,
            TagValue::Text(text(system)?),
        ));
    }
    if let Vcs::Git(url) = &package.vcs {
        tags.push((Tag::VCS, None, TagValue::Text(text(&format!("git:{url}"))?)));
    }
    for (number, source) in &manifest.sources {
        tags.push((
            Tag::Source(Some(*number)),
            None,
            TagValue::Text(text(source.value())?),
        ));
    }
    for (number, patch) in &manifest.patches {
        tags.push((
            Tag::Patch(Some(*number)),
            None,
            TagValue::Text(text(&patch.path)?),
        ));
    }
    for (stage, config) in &manifest.build.stages {
        for option in &config.options {
            // rpm-spec stores an unknown tag's parenthesized argument in `lang`.
            tags.push((
                Tag::Other("BuildOption".into()),
                Some(stage.as_str()),
                TagValue::Text(text(option)?),
            ));
        }
    }
    dependency_tags(
        &mut tags,
        Tag::BuildRequires,
        &manifest.build_requires.rpm,
        "build-requires.rpm",
    )?;
    tags.extend(body_tags(&package.body, "package")?);

    let mut patch_order = manifest.patches.iter().map(|(number, _)| *number);
    let mut comments = Vec::new();
    let mut sections = Vec::new();
    for (index, item) in parsed.spec.items.iter().enumerate() {
        match item {
            SpecItem::Preamble(item) => {
                if let Tag::Patch(Some(number)) = item.tag {
                    check(
                        patch_order.next() == Some(number),
                        "Patch application order",
                    )?;
                }
                let field = format!("{:?}", item.tag);
                match_tag(item, &mut tags, &field)?;
                if let Tag::Source(Some(number)) = item.tag
                    && let Some((_, Source::Remote { sha256, .. })) =
                        manifest.sources.iter().find(|(n, _)| *n == number)
                {
                    let expected = format!("{}\n", profile.remote_asset(sha256.as_deref()));
                    // The hook consumes exact bytes adjacent to this material, not just any matching comment.
                    let previous = index.checked_sub(1).and_then(|i| parsed.spec.items.get(i));
                    check(
                        matches!(previous, Some(SpecItem::Comment(comment))
                        if comment.style == CommentStyle::Hash
                        && source.get(comment.data.start_byte..comment.data.end_byte) == Some(expected.as_str())),
                        &format!("sources.{number}.sha256"),
                    )?;
                }
            }
            SpecItem::Comment(comment) => {
                check(comment.style == CommentStyle::Hash, "SPEC comments")?;
                if !comment.text.is_empty() {
                    comments.push(&comment.text);
                }
            }
            SpecItem::Section(section) => sections.push(section.as_ref()),
            SpecItem::Blank => {}
            _ => return Err(mismatch("top-level items")),
        }
    }
    check(tags.is_empty(), "missing main-package tags")?;

    let mut expected_comments = Vec::new();
    for holder in &profile.copyright_holders {
        expected_comments.push(text(&format!(
            "SPDX-FileCopyrightText: (C) {} {holder}",
            manifest.spec.copyright_years
        ))?);
    }
    for contributor in &manifest.spec.contributors {
        expected_comments.push(text(&format!("SPDX-FileContributor: {contributor}"))?);
    }
    // Keep the generated marker split so REUSE does not read it as this source file's license.
    expected_comments.push(text(
        &["SPDX-License-", "Identifier: ", &profile.spec_license].concat(),
    )?);
    if matches!(package.vcs, Vcs::NoPublicRepository) {
        expected_comments.push(comment_text(&profile.no_public_vcs_comment)?);
    }
    for (_, source) in &manifest.sources {
        if let Source::Remote { sha256, .. } = source {
            expected_comments.push(comment_text(&profile.remote_asset(sha256.as_deref()))?);
        }
    }
    check(
        comments.iter().copied().eq(expected_comments.iter()),
        "SPEC metadata",
    )?;

    // Consume the complete generated sequence. Extra sections are not ignored,
    // and every subpackage reference must retain its declared naming form.
    let mut sections = sections.into_iter();
    description(
        sections.next(),
        None,
        &package.body.description,
        "package.description",
    )?;
    for subpackage in &manifest.subpackages {
        let (name, reference, field) = subpackage_identity(&subpackage.name)?;
        let Some(Section::Package {
            name_arg, content, ..
        }) = sections.next()
        else {
            return Err(mismatch(&field));
        };
        check(*name_arg == name, &field)?;
        let mut tags = body_tags(&subpackage.body, &field)?;
        for item in content {
            match item {
                PreambleContent::Item(item) => {
                    match_tag(item, &mut tags, &format!("{field}.{:?}", item.tag))?;
                }
                PreambleContent::Blank => {}
                _ => return Err(mismatch(&field)),
            }
        }
        check(tags.is_empty(), &field)?;
        description(
            sections.next(),
            Some(&reference),
            &subpackage.body.description,
            &format!("{field}.description"),
        )?;
    }
    build_scripts(&mut sections, source, manifest)?;
    file_section(
        sections.next(),
        source,
        None,
        &package.body.files,
        "package.files",
    )?;
    for subpackage in &manifest.subpackages {
        let (_, reference, field) = subpackage_identity(&subpackage.name)?;
        file_section(
            sections.next(),
            source,
            Some(&reference),
            &subpackage.body.files,
            &format!("{field}.files"),
        )?;
    }
    let Some(Section::Changelog { items, .. }) = sections.next() else {
        return Err(mismatch("changelog"));
    };
    let expected_changelog = text(&profile.changelog)?;
    check(
        matches!((items.as_slice(), expected_changelog.segments.as_slice()),
        ([ChangelogItem::Statement { macro_ref, .. }], [TextSegment::Macro(expected)]) if macro_ref == expected.as_ref()),
        "changelog",
    )?;
    check(sections.next().is_none(), "unexpected sections")
}

fn body_tags(body: &PackageBody, field: &str) -> Result<Vec<ExpectedTag>, RenderError> {
    let mut tags = vec![(Tag::Summary, None, TagValue::Text(text(&body.summary)?))];
    dependency_tags(
        &mut tags,
        Tag::Requires,
        &body.requires,
        &format!("{field}.requires"),
    )?;
    dependency_tags(
        &mut tags,
        Tag::Provides,
        &body.provides,
        &format!("{field}.provides"),
    )?;
    Ok(tags)
}

fn dependency_tags(
    tags: &mut Vec<ExpectedTag>,
    tag: Tag,
    values: &[String],
    field: &str,
) -> Result<(), RenderError> {
    for expression in values {
        let state = ParserState::new();
        let value = parse_dep_expr(&state, expression).map_err(|()| mismatch(field))?;
        check(state.diagnostics.borrow().is_empty(), field)?;
        tags.push((tag.clone(), None, TagValue::Dep(value)));
    }
    Ok(())
}

fn match_tag(
    item: &PreambleItem<Span>,
    tags: &mut Vec<ExpectedTag>,
    field: &str,
) -> Result<(), RenderError> {
    check(item.qualifiers.is_empty(), field)?;
    let position = tags
        .iter()
        .position(|(tag, argument, _)| *tag == item.tag && *argument == item.lang.as_deref())
        .ok_or_else(|| mismatch(field))?;
    let (_, _, expected) = tags.remove(position);
    check(item.value == expected, field)
}

fn subpackage_identity(
    name: &SubpackageName,
) -> Result<(PackageName, SubpkgRef, String), RenderError> {
    match name {
        SubpackageName::Suffix(name) => {
            let value = text(name)?;
            Ok((
                PackageName::Relative(value.clone()),
                SubpkgRef::Relative(value),
                format!("subpackages.{name}"),
            ))
        }
        SubpackageName::Absolute(name) => {
            let value = text(name)?;
            Ok((
                PackageName::Absolute(value.clone()),
                SubpkgRef::Absolute(value),
                format!("subpackages.{name}"),
            ))
        }
    }
}

fn description(
    section: Option<&Section<Span>>,
    expected_subpackage: Option<&SubpkgRef>,
    expected: &str,
    field: &str,
) -> Result<(), RenderError> {
    let Some(Section::Description {
        subpkg: subpackage,
        body,
        ..
    }) = section
    else {
        return Err(mismatch(field));
    };
    check(subpackage.as_ref() == expected_subpackage, field)?;
    let mut lines: Vec<_> = expected.lines().collect();
    // The parser discards separator lines at the end, not indentation or prose spaces.
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let expected = lines.into_iter().map(text).collect::<Result<Vec<_>, _>>()?;
    check(body.lines == expected, field)
}

fn file_section(
    section: Option<&Section<Span>>,
    source: &str,
    expected_subpackage: Option<&SubpkgRef>,
    expected: &Files,
    field: &str,
) -> Result<(), RenderError> {
    let Some(Section::Files {
        subpkg: subpackage,
        file_lists,
        content,
        data,
    }) = section
    else {
        return Err(mismatch(field));
    };
    check(
        subpackage.as_ref() == expected_subpackage
            && *file_lists
                == expected
                    .lists
                    .iter()
                    .map(|s| text(s))
                    .collect::<Result<Vec<_>, _>>()?,
        field,
    )?;
    // The pinned parser overwrites earlier package arguments in a %files
    // header. Check the source header as well so those lost arguments cannot
    // make an invalid declaration look like the expected package reference.
    let header = source
        .get(data.start_byte..data.end_byte)
        .and_then(|section| section.lines().next())
        .ok_or_else(|| mismatch(field))?;
    let mut words = header.split_whitespace();
    check(words.next() == Some("%files"), field)?;
    if let Some(expected) = expected_subpackage {
        if matches!(expected, SubpkgRef::Absolute(_)) {
            check(words.next() == Some("-n"), field)?;
        }
        let name = words.next().ok_or_else(|| mismatch(field))?;
        let expected = match expected {
            SubpkgRef::Relative(t) | SubpkgRef::Absolute(t) => t,
            _ => return Err(mismatch(field)),
        };
        check(text(name)? == *expected, field)?;
    }
    for list in &expected.lists {
        check(
            words.next() == Some("-f") && words.next() == Some(list.as_str()),
            field,
        )?;
    }
    check(words.next().is_none(), field)?;
    files(content, source, expected, field)
}

fn build_scripts<'a>(
    sections: &mut impl Iterator<Item = &'a Section<Span>>,
    source: &str,
    manifest: &Manifest,
) -> Result<(), RenderError> {
    for (stage, config) in &manifest.build.stages {
        let expected_kind = match stage {
            Stage::Prep => BuildScriptKind::Prep,
            Stage::Conf => BuildScriptKind::Conf,
            Stage::Build => BuildScriptKind::Build,
            Stage::Install => BuildScriptKind::Install,
            Stage::Check => BuildScriptKind::Check,
        };
        for (name, expected_placement, script) in [
            (
                "prepend",
                BuildScriptPlacement::Prepend,
                Some(config.prepend.as_str()),
            ),
            (
                "replace",
                BuildScriptPlacement::Main,
                config.replace.as_deref(),
            ),
            (
                "append",
                BuildScriptPlacement::Append,
                Some(config.append.as_str()),
            ),
        ] {
            let Some(script) = script else {
                continue;
            };
            if script.is_empty() && expected_placement != BuildScriptPlacement::Main {
                continue;
            }
            let field = format!("build.stages.{}.{name}", stage.as_str());
            let Some(Section::BuildScript {
                kind,
                placement,
                data,
                ..
            }) = sections.next()
            else {
                return Err(mismatch(&field));
            };
            check(
                *kind == expected_kind && *placement == expected_placement,
                &field,
            )?;
            // Script whitespace can be significant, especially inside heredocs.
            // Compare source bytes: the AST omits trailing blank lines.
            let body = source
                .get(data.start_byte..data.end_byte)
                .and_then(|section| section.split_once('\n'))
                .map(|(_, body)| body)
                .ok_or_else(|| mismatch(&field))?;
            let separator = if script.is_empty() || script.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            check(
                body.strip_prefix(script)
                    .is_some_and(|tail| tail == separator),
                &field,
            )?;
        }
    }
    Ok(())
}

fn files(
    content: &[FilesContent<Span>],
    source: &str,
    files: &Files,
    field: &str,
) -> Result<(), RenderError> {
    let mut expected = Vec::new();
    for (directive, paths) in [
        (FileDirective::License, &files.license),
        (FileDirective::Doc, &files.doc),
    ] {
        if !paths.is_empty() {
            expected.push((vec![directive], Some(text(&paths.join(" "))?), None));
        }
    }
    for path in &files.entries {
        let entry = super::files::entry(path).map_err(mismatch)?;
        expected.push((
            entry.directives,
            entry.path.map(|p| p.path),
            Some(path.as_str()),
        ));
    }
    let mut expected = expected.into_iter();
    for item in content {
        match item {
            FilesContent::Blank => {}
            FilesContent::Entry(entry) => {
                let (directives, path, raw) = expected.next().ok_or_else(|| mismatch(field))?;
                // Unknown flag tokens can disappear from the rpm-spec AST.
                // Authored native rows must also retain their original bytes.
                if let Some(raw) = raw {
                    check(
                        source
                            .get(entry.data.start_byte..entry.data.end_byte)
                            .map(|s| s.trim_end_matches('\n'))
                            == Some(raw),
                        field,
                    )?;
                }
                check(
                    entry.directives == directives
                        && entry.path.as_ref().map(|p| &p.path) == path.as_ref(),
                    field,
                )?;
            }
            _ => return Err(mismatch(field)),
        }
    }
    check(expected.next().is_none(), field)
}

/// Parses expressions for comparison without evaluating or rewriting their macros.
fn text(value: &str) -> Result<Text, RenderError> {
    let state = ParserState::new();
    let (rest, parsed) = parse_text(&state, Input::new(value), &|_| false)
        .map_err(|_| mismatch("input expression"))?;
    check(
        rest.fragment().is_empty() && state.diagnostics.borrow().is_empty(),
        "input expression",
    )?;
    Ok(parsed)
}

fn comment_text(value: &str) -> Result<Text, RenderError> {
    let body = value
        .strip_prefix('#')
        .ok_or_else(|| mismatch("profile comment"))?;
    text(body.strip_prefix(' ').unwrap_or(body))
}

fn check(matches: bool, field: &str) -> Result<(), RenderError> {
    if matches {
        Ok(())
    } else {
        Err(mismatch(field))
    }
}

fn mismatch(field: &str) -> RenderError {
    RenderError::Invalid(format!(
        "generated SPEC does not match manifest/profile: {field}"
    ))
}

#[cfg(test)]
mod tests;
