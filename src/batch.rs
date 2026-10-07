// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Collect sequential results and retain unvisited inputs after cancellation.

use serde::Serialize;

/// Preserve completed results and the unvisited suffix when an operation cancels.
#[derive(Serialize)]
pub(crate) struct Report<'a, I, R> {
    operation: &'static str,
    pub(crate) success: bool,
    pub(crate) results: Vec<R>,
    pub(crate) pending: &'a [I],
}

pub(crate) fn run<'a, I, R>(
    operation: &'static str,
    items: &'a [I],
    mut execute: impl FnMut(&I) -> std::ops::ControlFlow<R, R>,
    succeeded: impl Fn(&R) -> bool,
) -> Report<'a, I, R> {
    use std::ops::ControlFlow::{Break, Continue};
    let mut results = Vec::new();
    for item in items {
        let step = execute(item);
        let stop = step.is_break();
        results.push(match step {
            Break(result) | Continue(result) => result,
        });
        if stop {
            break;
        }
    }
    let pending = &items[results.len()..];
    Report {
        operation,
        success: pending.is_empty() && results.iter().all(succeeded),
        results,
        pending,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn batch_keeps_failures_and_cancellation_before_the_pending_suffix() {
        use std::ops::ControlFlow::{Break, Continue};
        let items = [0, 1, 2, 3];
        let report = super::run(
            "test",
            &items,
            |&item| {
                if item == 2 {
                    Break(item)
                } else {
                    Continue(item)
                }
            },
            |&item| item == 1,
        );
        assert_eq!(report.results, [0, 1, 2]);
        assert_eq!(report.pending, [3]);
        assert!(!report.success);
    }
}
