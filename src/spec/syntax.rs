// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Field traversal without RPM evaluation.

use rpm_spec::ast::{PreambleItem, Span, SpecFile, Tag, TagValue};
use rpm_spec_analyzer::visit::Visit;

use crate::check::license::LicenseCheck;

pub(super) fn license(spec: &SpecFile<Span>) -> LicenseCheck {
    let mut visitor = LicenseVisitor(LicenseCheck::default());
    visitor.visit_spec(spec);
    visitor.0
}

struct LicenseVisitor(LicenseCheck);

impl<'ast> Visit<'ast> for LicenseVisitor {
    fn visit_preamble(&mut self, item: &'ast PreambleItem<Span>) {
        if item.tag == Tag::License {
            let literal = match &item.value {
                TagValue::Text(text) => text.literal_str(),
                _ => None,
            };
            self.0
                .check(literal, super::diagnostic::location(item.data));
        }
    }
}

pub(super) fn metadata(spec: &SpecFile<Span>) -> Vec<crate::check_report::Finding> {
    let mut visitor = MetadataVisitor(Vec::new());
    visitor.visit_spec(spec);
    visitor.0
}

struct MetadataVisitor(Vec<crate::check_report::Finding>);

impl<'ast> Visit<'ast> for MetadataVisitor {
    fn visit_preamble(&mut self, item: &'ast PreambleItem<Span>) {
        use crate::check::metadata::Field;
        let field = match item.tag {
            Tag::Name => Field::Name,
            Tag::Version => Field::Version,
            Tag::Release => Field::Release,
            Tag::URL => Field::Url,
            _ => return,
        };
        let literal = match &item.value {
            TagValue::Text(text) => text.literal_str(),
            _ => None,
        };
        if let Some(finding) = field.finding(literal, super::diagnostic::location(item.data)) {
            self.0.push(finding);
        }
    }
}

pub(super) fn build_requirements(spec: &SpecFile<Span>) -> crate::check::build::BuildRequirements {
    let mut visitor = BuildVisitor {
        facts: Default::default(),
        conditional: false,
    };
    visitor.visit_spec(spec);
    visitor.facts
}

struct BuildVisitor {
    facts: crate::check::build::BuildRequirements,
    conditional: bool,
}

impl<'ast> Visit<'ast> for BuildVisitor {
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
            SpecItem::Include(_) | SpecItem::Statement(_) => self.facts.uncertain = true,
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
        use rpm_spec::ast::DepExpr;
        if matches!(&item.tag, Tag::Other(name) if name.eq_ignore_ascii_case("BuildSystem")) {
            let system = match &item.value {
                TagValue::Text(text) if !self.conditional => {
                    text.literal_str().map(|s| s.trim().to_owned())
                }
                _ => None,
            };
            self.facts
                .systems
                .push((system, super::diagnostic::location(item.data)));
        } else if item.tag == Tag::BuildRequires {
            if !self.conditional
                && item.qualifiers.is_empty()
                && let TagValue::Dep(DepExpr::Atom(atom)) = &item.value
                && atom.arch.is_none()
                && let Some(name) = atom.name.literal_str()
            {
                self.facts.direct.push(name.to_owned());
            } else {
                self.facts.uncertain = true;
            }
        }
    }
}
