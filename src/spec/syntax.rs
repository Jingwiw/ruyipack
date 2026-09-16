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
