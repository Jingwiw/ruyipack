// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Source declarations and their static values share one ordered AST walk.
//! Macro execution, includes and unknown branch selection are never guessed.

use super::{ParsedSpec, expression::Context};
use rpm_spec::{
    ast::{CommentStyle, PreambleContent, Section, Span, SpecItem, Tag},
    parse_result::Severity,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

#[derive(Clone)]
pub(crate) struct Source {
    pub(crate) span: crate::source_location::SourceLocation,
    pub(crate) expression: String,
    pub(crate) digest: Result<Option<String>, String>,
    pub(crate) url: Result<String, String>,
}

#[derive(Clone)]
pub(crate) struct Resolution {
    pub(crate) sources: BTreeMap<u32, Source>,
    pub(crate) patches: BTreeMap<u32, Source>,
    // Known entries do not prove that unexecuted constructs declare no others.
    pub(crate) incomplete: Option<String>,
}

pub(crate) fn resolve<'a>(
    spec: &'a ParsedSpec<'_>,
    defines: &[String],
) -> Result<Cow<'a, Resolution>, String> {
    // Unchanged source and no overrides have one meaning. Explicit -D contexts
    // are separate evaluations and must never reuse the default-context facts.
    if defines.is_empty() {
        spec.sources
            .get_or_init(|| resolve_with_patches(spec, defines, false))
            .as_ref()
            .map(Cow::Borrowed)
            .map_err(Clone::clone)
    } else {
        resolve_with_patches(spec, defines, false).map(Cow::Owned)
    }
}

/// Include Patch declarations without changing the scope of existing Source operations.
pub(crate) fn resolve_materials(
    spec: &ParsedSpec<'_>,
    defines: &[String],
) -> Result<Resolution, String> {
    resolve_with_patches(spec, defines, true)
}

fn resolve_with_patches(
    spec: &ParsedSpec<'_>,
    defines: &[String],
    patches: bool,
) -> Result<Resolution, String> {
    if spec
        .parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
        return Err("parser errors prevent Source resolution".into());
    }
    let mut walk = Resolver {
        spec,
        context: Context::from_defines(defines)?,
        next: Some(0),
        sources: BTreeMap::new(),
        patches: BTreeMap::new(),
        next_patch: Some(0),
        include_patches: patches,
        seen: BTreeSet::new(),
        unknown: None,
    };
    walk.items(&spec.parsed.spec.items)?;
    Ok(Resolution {
        sources: walk.sources,
        patches: walk.patches,
        incomplete: walk.unknown,
    })
}

struct Resolver<'a> {
    spec: &'a ParsedSpec<'a>,
    context: Context,
    next: Option<u32>,
    sources: BTreeMap<u32, Source>,
    patches: BTreeMap<u32, Source>,
    next_patch: Option<u32>,
    include_patches: bool,
    unknown: Option<String>,
    seen: BTreeSet<&'static str>,
}

impl Resolver<'_> {
    fn items(&mut self, items: &[SpecItem<Span>]) -> Result<(), String> {
        for item in items {
            match item {
                SpecItem::MacroDef(definition) => {
                    if let Err(reason) = self.context.define(definition) {
                        self.unknown = Some(reason);
                        self.next = None;
                        self.next_patch = None;
                    }
                }
                SpecItem::Preamble(item) => {
                    let raw = self
                        .spec
                        .source
                        .get(item.data.start_byte..item.data.end_byte)
                        .ok_or("invalid Source AST range")?;
                    let expression = raw.split_once(':').ok_or("preamble has no colon")?.1.trim();
                    // The full AST may recover a malformed macro with only a warning.
                    // Strict expression parsing must not turn that recovery into a URL.
                    let value = self.context.expand_str(expression).and_then(|value| {
                        if value.contains(['\n', '\r', '\0']) {
                            Err("preamble expansion changes line structure".into())
                        } else {
                            Ok(value)
                        }
                    });
                    let material = match item.tag {
                        Tag::Source(number) => Some((false, number)),
                        Tag::Patch(number) if self.include_patches => Some((true, number)),
                        _ => None,
                    };
                    if let Some((patch, explicit)) = material {
                        let number = super::source_number(
                            if patch {
                                &mut self.next_patch
                            } else {
                                &mut self.next
                            },
                            explicit,
                        )
                        .ok_or(if patch {
                            "implicit Patch number is uncertain or overflowed"
                        } else {
                            "implicit Source number is uncertain or overflowed"
                        })?;
                        if item.lang.is_some() || !item.qualifiers.is_empty() {
                            return Err(format!(
                                "qualified {} declarations are unsupported",
                                if patch { "Patch" } else { "Source" }
                            ));
                        }
                        let url = self
                            .unknown
                            .as_ref()
                            .map_or(value, |reason| Err(reason.clone()));
                        let source = Source {
                            span: super::diagnostic::location(item.data),
                            expression: expression.to_owned(),
                            digest: self.digest(item.data.start_byte),
                            url,
                        };
                        let unresolved = source.url.as_ref().err().cloned();
                        if (if patch {
                            &mut self.patches
                        } else {
                            &mut self.sources
                        })
                        .insert(number, source)
                        .is_some()
                        {
                            return Err(format!(
                                "duplicate {}{number}",
                                if patch { "Patch" } else { "Source" }
                            ));
                        }
                        if let Some(reason) = unresolved {
                            self.unknown = Some(reason);
                            self.next = None;
                            self.next_patch = None;
                        }
                    } else {
                        // Unknown expansions can emit declarations or change macros. The
                        // shipped Release convention is emitted, not evaluated by this tool.
                        if let Err(reason) = &value
                            && !(matches!(item.tag, Tag::Release)
                                && expression == crate::profile::load().release)
                        {
                            self.unknown = Some(reason.clone());
                            self.next = None;
                            self.next_patch = None;
                        }
                        let name = match item.tag {
                            Tag::Name => Some("name"),
                            Tag::Version => Some("version"),
                            Tag::URL => Some("url"),
                            Tag::Release => Some("release"),
                            _ => None,
                        };
                        if let Some(name) = name {
                            let value = if !self.seen.insert(name)
                                || item.lang.is_some()
                                || !item.qualifiers.is_empty()
                            {
                                Err(format!("ambiguous package field {name}"))
                            } else {
                                value
                            };
                            self.context.literal(name, value);
                        }
                    }
                }
                SpecItem::Conditional(condition) => {
                    let mut selected = condition.otherwise.as_deref();
                    for branch in &condition.branches {
                        match self.context.condition(branch.kind, &branch.expr) {
                            Ok(true) => {
                                selected = Some(&branch.body);
                                break;
                            }
                            Ok(false) => {}
                            Err(reason) => {
                                let reason = format!(
                                    "line {}: conditional is unresolved: {reason}",
                                    condition.data.start_line
                                );
                                self.unknown = Some(reason.clone());
                                self.next = None;
                                self.next_patch = None;
                                for branch in &condition.branches {
                                    self.uncertain(&branch.body, &reason)?;
                                }
                                if let Some(body) = &condition.otherwise {
                                    self.uncertain(body, &reason)?;
                                }
                                selected = None;
                                break;
                            }
                        }
                    }
                    if let Some(body) = selected {
                        self.items(body)?;
                    }
                }
                SpecItem::Statement(reference)
                    if self
                        .context
                        .expand(&rpm_spec::ast::Text::from(reference.as_ref().clone()))
                        .is_ok_and(|value| value.is_empty()) => {}
                SpecItem::Include(_) | SpecItem::Statement(_) => {
                    self.unknown = Some("Source context is unavailable or ambiguous after an include or top-level invocation; not executed".into());
                    self.next = None;
                    self.next_patch = None;
                }
                SpecItem::Comment(comment)
                    if comment.style == CommentStyle::Hash
                        && comment.text.literal_str().is_none() =>
                {
                    // RPM expands # comments too. Inert known references are fine;
                    // unknown invocations must not silently change later URL meaning.
                    if !self
                        .context
                        .expand(&comment.text)
                        .is_ok_and(|text| !text.contains(['\n', '\r', '\0']))
                    {
                        self.unknown =
                            Some("macro-bearing comment has unknown effects; not executed".into());
                        self.next = None;
                        self.next_patch = None;
                    }
                }
                SpecItem::BuildCondition(condition) => {
                    if let Some(default) = &condition.default
                        && !self
                            .context
                            .expand(default)
                            .is_ok_and(|text| !text.contains(['\n', '\r', '\0']))
                    {
                        self.unknown =
                            Some("build condition default has unknown macro effects".into());
                        self.next = None;
                        self.next_patch = None;
                    }
                }
                SpecItem::Section(section) if matches!(section.as_ref(), Section::Package { content, .. } if subpackage_sources(content, self.include_patches)) =>
                {
                    return Err("Source declarations inside a subpackage are unsupported".into());
                }
                SpecItem::Section(section)
                    if matches!(section.as_ref(), Section::SourceList { .. })
                        || self.include_patches
                            && matches!(section.as_ref(), Section::PatchList { .. }) =>
                {
                    return Err(if matches!(section.as_ref(), Section::PatchList { .. }) {
                        "%patchlist is not supported by material resolution"
                    } else {
                        "%sourcelist is not supported by Source resolution"
                    }
                    .into());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn uncertain(&mut self, items: &[SpecItem<Span>], reason: &str) -> Result<(), String> {
        for item in items {
            match item {
                SpecItem::MacroDef(definition) => self
                    .context
                    .literal(&definition.name, Err(reason.to_owned())),
                SpecItem::Preamble(item) => match item.tag {
                    Tag::Source(_) => return Err(reason.to_owned()),
                    Tag::Patch(_) if self.include_patches => return Err(reason.to_owned()),
                    Tag::Name => self.context.literal("name", Err(reason.to_owned())),
                    Tag::Version => self.context.literal("version", Err(reason.to_owned())),
                    Tag::URL => self.context.literal("url", Err(reason.to_owned())),
                    _ => {}
                },
                SpecItem::Conditional(condition) => {
                    for branch in &condition.branches {
                        self.uncertain(&branch.body, reason)?;
                    }
                    if let Some(body) = &condition.otherwise {
                        self.uncertain(body, reason)?;
                    }
                }
                SpecItem::Include(_) | SpecItem::Statement(_) => {
                    self.unknown = Some(reason.to_owned());
                    self.next = None;
                    self.next_patch = None;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn digest(&self, offset: usize) -> Result<Option<String>, String> {
        let before = &self.spec.source[..offset];
        let previous = before
            .strip_suffix('\n')
            .unwrap_or(before)
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .trim_end_matches('\r');
        let profile = crate::profile::load();
        if !previous.starts_with(&profile.remote_asset_bare) {
            return Ok(None);
        }
        profile
            .remote_asset_digest(previous)
            .map(|hash| hash.map(str::to_owned))
            .map_err(str::to_owned)
    }
}

fn subpackage_sources(items: &[PreambleContent<Span>], patches: bool) -> bool {
    items.iter().any(|item| match item {
        PreambleContent::Item(item) => {
            matches!(item.tag, Tag::Source(_)) || patches && matches!(item.tag, Tag::Patch(_))
        }
        PreambleContent::Conditional(condition) => {
            condition
                .branches
                .iter()
                .any(|branch| subpackage_sources(&branch.body, patches))
                || condition
                    .otherwise
                    .as_ref()
                    .is_some_and(|body| subpackage_sources(body, patches))
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_static_facts_never_guess_dynamic_sources() {
        let prefix = "Name: probe\nVersion: 1\nRelease: 1\nSummary: Probe\nLicense: MIT\n";
        let parsed = ParsedSpec::parse("Source0: https://example.org/%{target}\n");
        for target in [None, Some("first"), Some("second"), None] {
            let defines = target
                .map(|value| format!("target {value}"))
                .into_iter()
                .collect::<Vec<_>>();
            let resolved = resolve(&parsed, &defines).unwrap();
            assert_eq!(
                resolved.sources[&0].url.as_ref().ok(),
                target
                    .map(|value| format!("https://example.org/{value}"))
                    .as_ref()
            );
        }
        for row in include_str!("../../tests/fixtures/source-expressions.tsv")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let columns = row.split('\t').collect::<Vec<_>>();
            let definitions = columns[0].replace("\\n", "\n");
            let source = format!(
                "{prefix}{definitions}Source0: {}\n%description\nProbe\n",
                columns[1]
            );
            let defines = if columns[3].is_empty() {
                vec![]
            } else {
                vec![columns[3].to_owned()]
            };
            assert_eq!(
                resolve(&ParsedSpec::parse(&source), &defines)
                    .unwrap()
                    .sources[&0]
                    .url
                    .as_deref(),
                Ok(columns[2]),
                "{row}"
            );
        }
        for definitions in [
            "%define v %{v}\n",
            "%global v %(touch SHOULD_NOT_EXIST)\n",
            "%global v %{lua:return 'unsafe'}\n",
            "%if %{missing}\n%global v guessed\n%endif\n",
            "%include absent.inc\n",
        ] {
            let source = format!("{prefix}{definitions}Source0: https://example.org/%{{v}}\n");
            assert!(
                resolve(&ParsedSpec::parse(&source), &[]).unwrap().sources[&0]
                    .url
                    .is_err(),
                "{definitions}"
            );
        }
        let source = format!(
            "{prefix}Source3: https://example.org/3\nSource: https://example.org/4\nSource1: local.tar\nSource: https://example.org/5\n"
        );
        assert_eq!(
            resolve(&ParsedSpec::parse(&source), &[])
                .unwrap()
                .sources
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [1, 3, 4, 5]
        );
        for header in ["Summary: %{injected}", "# %{injected}"] {
            let source = format!("{prefix}{header}\nSource0: https://example.org/archive\n");
            let parsed = ParsedSpec::parse(&source);
            let result = resolve(&parsed, &["injected text\nSource1: hidden".into()]).unwrap();
            assert!(result.incomplete.is_some(), "{header}");
            assert!(result.sources[&0].url.is_err());
        }
        let parsed = ParsedSpec::parse("%include external.inc\n");
        let unknown = resolve(&parsed, &[]).unwrap();
        assert!(unknown.sources.is_empty() && unknown.incomplete.is_some());
        let hidden = format!(
            "{prefix}%package extras\nSummary: Extra\nSource0: https://example.org/hidden\n"
        );
        assert!(resolve(&ParsedSpec::parse(&hidden), &[]).is_err());
        let source = format!(
            "{prefix}%ifarch aarch64\nSource0: https://example.org/arm\n%else\nSource0: https://example.org/other\n%endif\n"
        );
        assert!(resolve(&ParsedSpec::parse(&source), &[]).is_err());
        assert_eq!(
            resolve(&ParsedSpec::parse(&source), &["_target_cpu aarch64".into()])
                .unwrap()
                .sources[&0]
                .url
                .as_deref(),
            Ok("https://example.org/arm")
        );
    }
}
