// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! CLI boundaries share one executable; each domain remains independently filterable.

mod checks;
mod cli;
mod diagnostic_locations;
mod edit;
mod generation;
mod http;
mod init;
mod inspect;
mod output;
mod source_hash;
mod source_validation;
mod support;
