// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! License tag traversal without RPM evaluation.

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
