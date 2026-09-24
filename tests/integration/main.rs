// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! CLI boundaries share one executable; each domain remains independently filterable.

mod cli;
mod diagnostic_locations;
mod edit;
mod edit_selected;
mod edit_source_selection;
mod generation;
mod init;
mod inspect_json;
mod output;
mod source_validation;
mod support;
mod validation;
