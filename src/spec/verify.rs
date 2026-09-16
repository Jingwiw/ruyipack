// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Checks generated facts against the manifest and distribution defaults.

use super::ParsedSpec;
use crate::profile::Profile;
use crate::render::{
    RenderError,
    manifest::{Manifest, Stage, Vcs},
};
use rpm_spec::{
    ast::{
        BuildScriptKind, BuildScriptPlacement, ChangelogItem, CommentStyle, FileDirective,
        FilesContent, Section, Span, SpecItem, Tag, TagValue, Text, TextSegment,
    },
    parser::{Input, ParserState, deps::parse_dep_expr, text::parse_text},
};

/// Checks candidate facts against the manifest and profile.
pub(crate) fn run(
    spec: &ParsedSpec<'_>,
    recipe: &Manifest,
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

    let package = &recipe.package;
    let mut tags = Vec::new();
    for (tag, value) in [
        (Tag::Name, package.name.as_str()),
        (Tag::Version, &package.version),
        (Tag::Release, &profile.release),
        (Tag::Summary, &package.summary),
        (Tag::License, &package.license),
        (Tag::URL, &package.url),
    ] {
        tags.push((tag, None, TagValue::Text(text(value)?)));
    }
    if let Some(system) = &recipe.build.system {
        tags.push((
            Tag::Other("BuildSystem".into()),
            None,
            TagValue::Text(text(system)?),
        ));
    }
    if let Vcs::Git(url) = &package.vcs {
        tags.push((Tag::VCS, None, TagValue::Text(text(&format!("git:{url}"))?)));
    }
    for (number, source) in &recipe.sources {
        tags.push((
            Tag::Source(Some(*number)),
            None,
            TagValue::Text(text(&source.url)?),
        ));
    }
    for (stage, config) in &recipe.build.stages {
        for option in &config.options {
            // rpm-spec stores an unknown tag's parenthesized argument in `lang`.
            tags.push((
                Tag::Other("BuildOption".into()),
                Some(stage.as_str()),
                TagValue::Text(text(option)?),
            ));
        }
    }
    for requirement in &recipe.build_requires.rpm {
        let state = ParserState::new();
        let value =
            parse_dep_expr(&state, requirement).map_err(|()| mismatch("build-requires.rpm"))?;
        check(state.diagnostics.borrow().is_empty(), "build-requires.rpm")?;
        tags.push((Tag::BuildRequires, None, TagValue::Dep(value)));
    }

    let mut comments = Vec::new();
    let mut sections = Vec::new();
    for (index, item) in parsed.spec.items.iter().enumerate() {
        match item {
            SpecItem::Preamble(item) => {
                let field = format!("{:?}", item.tag);
                check(item.qualifiers.is_empty(), &field)?;
                let position = tags
                    .iter()
                    .position(|(tag, argument, _)| {
                        *tag == item.tag && *argument == item.lang.as_deref()
                    })
                    .ok_or_else(|| mismatch(&field))?;
                let (_, _, expected) = tags.remove(position);
                check(item.value == expected, &field)?;
                if let Tag::Source(Some(number)) = item.tag {
                    let expected = format!(
                        "{}{}\n",
                        profile.remote_asset_prefix, recipe.sources[&number].sha256
                    );
                    // A correct digest on a different Source line is still the wrong source identity.
                    let previous = index.checked_sub(1).and_then(|i| parsed.spec.items.get(i));
                    // The hook reads exact marker bytes; the AST removes optional comment whitespace.
                    check(
                        matches!(previous, Some(SpecItem::Comment(comment))
                        if comment.style == CommentStyle::Hash
                            && source.get(comment.data.start_byte..comment.data.end_byte)
                                == Some(expected.as_str())),
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
            recipe.spec.copyright_years
        ))?);
    }
    for contributor in &recipe.spec.contributors {
        expected_comments.push(text(&format!("SPDX-FileContributor: {contributor}"))?);
    }
    // Keep the generated marker split so REUSE does not read it as this source file's license.
    expected_comments.push(text(
        &["SPDX-License-", "Identifier: ", &profile.spec_license].concat(),
    )?);
    if matches!(package.vcs, Vcs::NoPublicRepository) {
        expected_comments.push(hash_comment(&profile.no_public_vcs_comment)?);
    }
    for source in recipe.sources.values() {
        expected_comments.push(hash_comment(&format!(
            "{}{}",
            profile.remote_asset_prefix, source.sha256
        ))?);
    }
    check(
        comments.iter().copied().eq(expected_comments.iter()),
        "SPEC metadata",
    )?;

    let [
        Section::Description {
            subpkg: None, body, ..
        },
        scripts @ ..,
        Section::Files {
            subpkg: None,
            file_lists,
            content,
            ..
        },
        Section::Changelog { items, .. },
    ] = sections.as_slice()
    else {
        return Err(mismatch("sections"));
    };
    let mut lines: Vec<_> = package.description.lines().collect();
    // The parser discards separator lines at the end, not indentation or spaces in prose.
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    let expected_body = lines.into_iter().map(text).collect::<Result<Vec<_>, _>>()?;
    check(body.lines == expected_body, "package.description")?;
    build_scripts(scripts, source, recipe)?;
    check(file_lists.is_empty(), "package.files")?;
    files(content, recipe)?;
    let expected_changelog = text(&profile.changelog)?;
    check(
        matches!((items.as_slice(), expected_changelog.segments.as_slice()),
        ([ChangelogItem::Statement { macro_ref, .. }], [TextSegment::Macro(expected)]) if macro_ref == expected.as_ref()),
        "changelog",
    )?;
    Ok(())
}

fn build_scripts(
    sections: &[&Section<Span>],
    source: &str,
    recipe: &Manifest,
) -> Result<(), RenderError> {
    let mut sections = sections.iter();
    for (stage, config) in &recipe.build.stages {
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
    check(sections.next().is_none(), "unexpected build scripts")
}

fn files(content: &[FilesContent<Span>], recipe: &Manifest) -> Result<(), RenderError> {
    let files = &recipe.package.files;
    let mut expected = Vec::new();
    for (directive, paths) in [
        (FileDirective::License, &files.license),
        (FileDirective::Doc, &files.doc),
    ] {
        if !paths.is_empty() {
            expected.push((vec![directive], text(&paths.join(" "))?));
        }
    }
    for path in &files.entries {
        expected.push((Vec::new(), text(path)?));
    }
    let mut expected = expected.into_iter();
    for item in content {
        match item {
            FilesContent::Blank => {}
            FilesContent::Entry(entry) => {
                let (directives, path) =
                    expected.next().ok_or_else(|| mismatch("package.files"))?;
                check(
                    entry.directives == directives
                        && entry
                            .path
                            .as_ref()
                            .is_some_and(|actual| actual.path == path),
                    "package.files",
                )?;
            }
            _ => return Err(mismatch("package.files")),
        }
    }
    check(expected.next().is_none(), "package.files")
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

fn hash_comment(value: &str) -> Result<Text, RenderError> {
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
