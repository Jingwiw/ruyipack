// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
//
// SPDX-License-Identifier: MulanPSL-2.0

//! Byte ranges and line positions for diagnostic output.

use std::ops::Range;

use serde::{Serialize, Serializer, ser::SerializeStruct};

/// Half-open byte range and 1-based (line, byte-column) endpoints.
///
/// Zero endpoints retain an upstream diagnostic with no line information.
#[derive(Clone)]
pub(crate) struct SourceLocation {
    pub(crate) bytes: Range<usize>,
    pub(crate) start: (u32, u32),
    pub(crate) end: (u32, u32),
}

impl Serialize for SourceLocation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut span = serializer.serialize_struct("Span", 6)?;
        span.serialize_field("start_byte", &self.bytes.start)?;
        span.serialize_field("end_byte", &self.bytes.end)?;
        span.serialize_field("start_line", &self.start.0)?;
        span.serialize_field("start_column", &self.start.1)?;
        span.serialize_field("end_line", &self.end.0)?;
        span.serialize_field("end_column", &self.end.1)?;
        span.end()
    }
}
