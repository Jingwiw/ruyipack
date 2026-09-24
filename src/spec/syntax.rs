// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Field traversal without RPM evaluation.

use rpm_spec::ast::{PreambleItem, Span, SpecFile, SpecItem, Tag, TagValue};
use rpm_spec_analyzer::visit::Visit;

use crate::check::{RuleResult, build::BuildRequirements, license};

pub(super) fn check(spec: &SpecFile<Span>, source: &str) -> RuleResult {
    let mut visitor = CheckVisitor {
        result: RuleResult::default(),
        build: BuildRequirements::default(),
        conditional: false,
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
    let build = visitor.build.check();
    visitor.result.findings.extend(build.findings);
    visitor
        .result
        .incomplete_reasons
        .extend(build.incomplete_reasons);
    visitor.result
}

struct CheckVisitor {
    result: RuleResult,
    build: BuildRequirements,
    conditional: bool,
}

impl<'ast> Visit<'ast> for CheckVisitor {
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
        match &item.tag {
            Tag::License => license::check(&mut self.result, "package.license", literal, span),
            Tag::Other(name) if name.eq_ignore_ascii_case("BuildSystem") => {
                let system = literal
                    .filter(|_| !self.conditional)
                    .map(|s| s.trim().to_owned());
                self.build.systems.push((system, span));
            }
            Tag::BuildRequires => {
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
    }
}
