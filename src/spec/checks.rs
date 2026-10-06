// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! openRuyi policy checks over literal fields, without RPM evaluation.

use rpm_spec::ast::{PreambleItem, Span, SpecItem, Tag, TagValue};
use rpm_spec_analyzer::visit::Visit;

use crate::check::{RuleResult, build::BuildRequirements, license};

pub(super) fn run(parsed: &super::ParsedSpec<'_>, defines: &[String]) -> RuleResult {
    let spec = &parsed.parsed.spec;
    let source = parsed.source();
    let mut visitor = CheckVisitor {
        source,
        result: RuleResult::default(),
        build: BuildRequirements::default(),
        conditional: false,
        subpackage: false,
        build_unproven: false,
    };
    visitor.visit_spec(spec);
    // Match the top-level comments exposed as spec.license by header editing,
    // not declaration-like text inside descriptions or shell bodies.
    for item in &spec.items {
        if let SpecItem::Comment(comment) = item
            && let Some(raw) = source.get(comment.data.start_byte..comment.data.end_byte)
            && let Some(value) =
                crate::spec_metadata::license_declaration(raw.strip_suffix('\n').unwrap_or(raw))
        {
            license::check(
                &mut visitor.result,
                "spec.license",
                Some(value),
                super::diagnostic::location(comment.data),
            );
        }
    }
    let mut build = visitor.build.check();
    if visitor.build_unproven {
        for finding in &mut build.findings {
            finding.rule_inputs = None;
        }
    }
    visitor.result.findings.extend(build.findings);
    visitor
        .result
        .incomplete_reasons
        .extend(build.incomplete_reasons);
    super::policy_fix::check(parsed, &mut visitor.result);
    check_sources(parsed, defines, &mut visitor.result);
    visitor.result
}

/// Share ordered macro/conditional facts with hashing and verification. Unknown
/// context is not evidence that a digest is missing, or that no Sources exist.
fn check_sources(parsed: &super::ParsedSpec<'_>, defines: &[String], result: &mut RuleResult) {
    let resolved = match super::sources::resolve(parsed, defines) {
        Ok(resolved) => resolved,
        Err(reason) => {
            result.source_uncertainty = Some(reason);
            return;
        }
    };
    result.source_uncertainty.clone_from(&resolved.incomplete);
    for (number, source) in &resolved.sources {
        let remote = match &source.url {
            Ok(url) => url.split_once(':').is_some_and(|(scheme, _)| {
                scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")
            }),
            Err(reason) => {
                result
                    .source_uncertainty
                    .get_or_insert_with(|| reason.clone());
                false
            }
        };
        let problem = match &source.digest {
            Err(reason) => Some(reason.as_str()),
            Ok(Some(hash)) if crate::source::validate_sha256(hash).is_err() => {
                Some("invalid sha256; expected 64 hexadecimal digits")
            }
            Ok(None) if remote => Some("no sha256; openRuyi requires SHA-256 for HTTP(S) sources"),
            _ => None,
        };
        if let Some(problem) = problem {
            let rule = crate::check::SOURCE_DIGEST_RULE;
            result.findings.push(crate::check_report::Finding {
                build_requirements: None,
                producer: "ruyipack",
                code: rule.code,
                severity: rule.severity,
                span: source.span.clone(),
                message: format!("Source{number}: {problem}"),
                rule_inputs: None,
            });
        }
    }
}

struct CheckVisitor<'a> {
    source: &'a str,
    result: RuleResult,
    build: BuildRequirements,
    conditional: bool,
    subpackage: bool,
    build_unproven: bool,
}

impl<'ast> Visit<'ast> for CheckVisitor<'_> {
    fn visit_file_entry(&mut self, entry: &'ast rpm_spec::ast::FileEntry<Span>) {
        let Some(raw) = self.source.get(entry.data.start_byte..entry.data.end_byte) else {
            return;
        };
        if ["%{_libdir}/pkgconfig/*", "%{_datadir}/pkgconfig/*"]
            .iter()
            .any(|prefix| raw.trim_start().starts_with(prefix))
        {
            let rule = crate::check::FILE_LIST_RULE;
            self.result.findings.push(crate::check_report::Finding {
                producer: "ruyipack", code: rule.code, severity: rule.severity,
                span: super::diagnostic::location(entry.data),
                message: "list installed pkg-config files explicitly; automatic repair requires an installed file list".into(),
                rule_inputs: None, build_requirements: None,
            });
        }
    }
    fn visit_section(&mut self, section: &'ast rpm_spec::ast::Section<Span>) {
        self.build.uncertain |= matches!(
            section,
            rpm_spec::ast::Section::BuildScript {
                kind: rpm_spec::ast::BuildScriptKind::GenerateBuildRequires,
                ..
            }
        );
        let previous = self.subpackage;
        self.subpackage |= matches!(section, rpm_spec::ast::Section::Package { .. });
        rpm_spec_analyzer::visit::walk_section(self, section);
        self.subpackage = previous;
    }

    fn visit_item(&mut self, item: &'ast rpm_spec::ast::SpecItem<Span>) {
        use rpm_spec::ast::SpecItem;
        match item {
            SpecItem::Conditional(_) => {
                let previous = self.conditional;
                self.conditional = true;
                rpm_spec_analyzer::visit::walk_item(self, item);
                self.conditional = previous;
            }
            // These constructs can supply tags absent from the static tree.
            SpecItem::Include(_) | SpecItem::Statement(_) => self.build.uncertain = true,
            _ => rpm_spec_analyzer::visit::walk_item(self, item),
        }
    }

    fn visit_preamble_conditional(
        &mut self,
        item: &'ast rpm_spec::ast::Conditional<Span, rpm_spec::ast::PreambleContent<Span>>,
    ) {
        let previous = self.conditional;
        self.conditional = true;
        rpm_spec_analyzer::visit::walk_preamble_conditional(self, item);
        self.conditional = previous;
    }

    fn visit_preamble(&mut self, item: &'ast PreambleItem<Span>) {
        use crate::check::metadata::Field;
        use rpm_spec::ast::DepExpr;
        let literal = match &item.value {
            TagValue::Text(text) => text.literal_str(),
            _ => None,
        };
        let span = super::diagnostic::location(item.data);
        let first_finding = self.result.findings.len();
        match &item.tag {
            Tag::License => license::check(&mut self.result, "package.license", literal, span),
            Tag::Other(name) if name.eq_ignore_ascii_case("BuildSystem") => {
                self.build_unproven |= self.subpackage;
                let system = literal
                    .filter(|_| !self.conditional)
                    .map(|s| s.trim().to_owned());
                self.build.systems.push((system, span));
            }
            Tag::BuildRequires => {
                self.build_unproven |= self.subpackage;
                if !self.conditional
                    && item.qualifiers.is_empty()
                    && let TagValue::Dep(DepExpr::Atom(atom)) = &item.value
                    && atom.arch.is_none()
                    && let Some(name) = atom.name.literal_str()
                {
                    self.build.direct.push(name.to_owned());
                } else {
                    self.build.uncertain = true;
                }
            }
            tag => {
                let field = match tag {
                    Tag::Name => Field::Name,
                    Tag::Summary => Field::Summary,
                    Tag::Version => Field::Version,
                    Tag::Release => Field::Release,
                    Tag::URL => Field::Url,
                    _ => return,
                };
                if let Some(finding) = field.finding(literal, span) {
                    self.result.findings.push(finding);
                }
            }
        }
        // The lexical rules share field names across branches and subpackages;
        // those names alone cannot prove that a retained error has the same owner.
        if self.conditional || self.subpackage {
            for finding in &mut self.result.findings[first_finding..] {
                finding.rule_inputs = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkgconfig_wildcards_need_artifact_evidence_not_guessed_names() {
        let source =
            "Name: pkg\n%description\n%{_libdir}/pkgconfig/*\n%files\n%{_libdir}/pkgconfig/*\n";
        let parsed = super::super::ParsedSpec::parse(source);
        let result = run(&parsed, &[]);
        let findings: Vec<_> = result
            .findings
            .iter()
            .filter(|f| f.code == "RPK008")
            .collect();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].span.start.0, 5);
        assert!(super::super::policy_fix::apply(&parsed).unwrap().is_none());
        let explicit = source.replace(
            "%files\n%{_libdir}/pkgconfig/*",
            "%files\n%{_libdir}/pkgconfig/pkg.pc",
        );
        assert!(
            !run(&super::super::ParsedSpec::parse(explicit), &[])
                .findings
                .iter()
                .any(|f| f.code == "RPK008")
        );
    }
}
