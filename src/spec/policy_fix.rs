// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Narrow repairs for openRuyi's changelog and declaration spacing rules.
use super::ParsedSpec;
use crate::{
    check::RuleResult,
    check_report::{Finding, SelectedRule, Severity},
};
use rpm_spec::ast::{PreambleItem, Section, Span};
use rpm_spec_analyzer::visit::{Visit, walk_section};

pub(crate) const RULE: SelectedRule = SelectedRule {
    code: "RPK007",
    severity: Severity::Warn,
};

struct Change {
    span: Span,
    replacement: String,
    field: &'static str,
}
struct Scan<'a> {
    source: &'a str,
    changes: Vec<Change>,
}

impl Scan<'_> {
    fn replace(&mut self, span: Span, replacement: String, field: &'static str) {
        self.changes.push(Change {
            span,
            replacement,
            field,
        });
    }
}
impl<'ast> Visit<'ast> for Scan<'_> {
    fn visit_preamble(&mut self, node: &'ast PreambleItem<Span>) {
        let span = node.data;
        let Some(raw) = self.source.get(span.start_byte..span.end_byte) else {
            return;
        };
        let Some((tag, value)) = raw.split_once(':') else {
            return;
        };
        // These are parsed declarations, not matching text in a shell or description.
        if (tag == "BuildRequires" || (tag.starts_with("BuildOption(") && tag.ends_with(')')))
            && !value.starts_with("  ")
            && !value.trim().is_empty()
        {
            self.replace(
                span,
                format!("{tag}:  {}", value.trim_start_matches([' ', '\t'])),
                "spec.spacing",
            );
        }
    }
    fn visit_section(&mut self, node: &'ast Section<Span>) {
        match node {
            Section::Files { data, .. } => {
                if let Some(raw) = self.source.get(data.start_byte..data.end_byte)
                    && let Some(tail) = raw.strip_prefix("%files")
                    && tail.starts_with("  ")
                    && tail
                        .trim_start_matches(' ')
                        .chars()
                        .next()
                        .is_some_and(|c| !c.is_whitespace())
                {
                    self.replace(
                        *data,
                        format!("%files {}", tail.trim_start_matches(' ')),
                        "spec.spacing",
                    );
                }
            }
            Section::Changelog { data, .. } => {
                if let Some(raw) = self.source.get(data.start_byte..data.end_byte)
                    && let Some((header, body)) = raw.split_once('\n')
                    && header.trim_end() == "%changelog"
                    && matches!(body.trim(), "%{?autochangelog}" | "%{autochangelog}")
                    && let Some(offset) = raw.find(body.trim())
                {
                    let mut replacement = raw.to_owned();
                    replacement.replace_range(offset..offset + body.trim().len(), "%autochangelog");
                    self.replace(*data, replacement, "spec.changelog");
                }
            }
            _ => {}
        }
        walk_section(self, node);
    }
}
fn changes(parsed: &ParsedSpec<'_>) -> Vec<Change> {
    let mut scan = Scan {
        source: parsed.source(),
        changes: vec![],
    };
    scan.visit_spec(&parsed.parsed.spec);
    scan.changes.sort_by_key(|c| c.span.start_byte);
    scan.changes
}
pub(super) fn check(parsed: &ParsedSpec<'_>, result: &mut RuleResult) {
    for change in changes(parsed) {
        result.findings.push(Finding {
            build_requirements: None,
            producer: "ruyipack",
            code: RULE.code,
            severity: RULE.severity,
            span: super::diagnostic::location(change.span),
            message: match change.field {
                "spec.changelog" => "use %autochangelog without a conditional macro wrapper",
                _ => "use openRuyi declaration spacing",
            }
            .into(),
            rule_inputs: None,
        });
    }
}
pub(crate) fn apply(
    parsed: &ParsedSpec<'_>,
) -> Result<Option<(ParsedSpec<'static>, Vec<String>)>, String> {
    let changes = changes(parsed);
    if changes.is_empty() {
        return Ok(None);
    }
    let mut output = String::with_capacity(parsed.source().len());
    let mut cursor = 0;
    let mut fields = Vec::new();
    for change in changes {
        if change.span.start_byte < cursor {
            return Err("overlapping policy repair ranges".into());
        }
        output.push_str(
            parsed
                .source()
                .get(cursor..change.span.start_byte)
                .ok_or("invalid policy repair range")?,
        );
        output.push_str(&change.replacement);
        cursor = change.span.end_byte;
        if !fields.iter().any(|f| f == change.field) {
            fields.push(change.field.to_owned());
        }
    }
    output.push_str(
        parsed
            .source()
            .get(cursor..)
            .ok_or("invalid policy repair end")?,
    );
    let candidate = ParsedSpec::parse(output);
    if candidate
        .parsed
        .diagnostics
        .iter()
        .any(|d| d.severity == rpm_spec::parse_result::Severity::Error)
    {
        return Err("policy repair produced an invalid SPEC".into());
    }
    Ok(Some((candidate, fields)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repairs_parsed_declarations_and_is_idempotent() {
        let source = "Name: pkg\nVersion: 1\nRelease: %autorelease\nSummary: Test\nLicense: MIT\nBuildRequires: make\nBuildOption(conf): --enable-shared\n%description\nBuildRequires: keep prose\n%files  devel\n/usr/lib/libfoo.so\n%changelog\n%{?autochangelog}\n";
        let expected = source
            .replacen("BuildRequires: make", "BuildRequires:  make", 1)
            .replace("BuildOption(conf): ", "BuildOption(conf):  ")
            .replace("%files  devel", "%files devel")
            .replace("%{?autochangelog}", "%autochangelog");
        let original = ParsedSpec::parse(source);
        let mut findings = RuleResult::default();
        check(&original, &mut findings);
        assert_eq!(findings.findings.len(), 4);
        let (fixed, _) = apply(&original).unwrap().unwrap();
        assert_eq!(fixed.source(), expected);
        assert!(apply(&fixed).unwrap().is_none());
    }
    #[test]
    fn does_not_erase_manual_changelog_or_macro_definition() {
        for body in [
            "%{?autochangelog}\n# Keep this note",
            "* Mon Jan 01 2024 Author <a@example.org> - 1-1\n- Keep history",
        ] {
            let source =
                format!("Name: pkg\n%description\n%{{?autochangelog}}\n%changelog\n{body}\n");
            assert!(apply(&ParsedSpec::parse(source)).unwrap().is_none());
        }
    }
}
