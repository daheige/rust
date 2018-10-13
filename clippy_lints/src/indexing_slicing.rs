// Copyright 2014-2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.


//! lint on indexing and slicing operations

use crate::consts::{constant, Constant};
use crate::utils;
use crate::utils::higher;
use crate::utils::higher::Range;
use crate::rustc::hir::*;
use crate::rustc::lint::{LateContext, LateLintPass, LintArray, LintPass};
use crate::rustc::{declare_tool_lint, lint_array};
use crate::rustc::ty;
use crate::syntax::ast::RangeLimits;

/// **What it does:** Checks for out of bounds array indexing with a constant
/// index.
///
/// **Why is this bad?** This will always panic at runtime.
///
/// **Known problems:** Hopefully none.
///
/// **Example:**
/// ```rust
/// let x = [1,2,3,4];
///
/// // Bad
/// x[9];
/// &x[2..9];
///
/// // Good
/// x[0];
/// x[3];
/// ```
declare_clippy_lint! {
    pub OUT_OF_BOUNDS_INDEXING,
    correctness,
    "out of bounds constant indexing"
}

/// **What it does:** Checks for usage of indexing or slicing. Arrays are special cased, this lint
/// does report on arrays if we can tell that slicing operations are in bounds and does not
/// lint on constant `usize` indexing on arrays because that is handled by rustc's `const_err` lint.
///
/// **Why is this bad?** Indexing and slicing can panic at runtime and there are
/// safe alternatives.
///
/// **Known problems:** Hopefully none.
///
/// **Example:**
/// ```rust
/// // Vector
/// let x = vec![0; 5];
///
/// // Bad
/// x[2];
/// &x[2..100];
/// &x[2..];
/// &x[..100];
///
/// // Good
/// x.get(2);
/// x.get(2..100);
/// x.get(2..);
/// x.get(..100);
///
/// // Array
/// let y = [0, 1, 2, 3];
///
/// // Bad
/// &y[10..100];
/// &y[10..];
/// &y[..100];
///
/// // Good
/// &y[2..];
/// &y[..2];
/// &y[0..3];
/// y.get(10);
/// y.get(10..100);
/// y.get(10..);
/// y.get(..100);
/// ```
declare_clippy_lint! {
    pub INDEXING_SLICING,
    restriction,
    "indexing/slicing usage"
}

#[derive(Copy, Clone)]
pub struct IndexingSlicing;

impl LintPass for IndexingSlicing {
    fn get_lints(&self) -> LintArray {
        lint_array!(INDEXING_SLICING, OUT_OF_BOUNDS_INDEXING)
    }
}

impl<'a, 'tcx> LateLintPass<'a, 'tcx> for IndexingSlicing {
    fn check_expr(&mut self, cx: &LateContext<'a, 'tcx>, expr: &'tcx Expr) {
        if let ExprKind::Index(ref array, ref index) = &expr.node {
            let ty = cx.tables.expr_ty(array);
            if let Some(range) = higher::range(cx, index) {
                // Ranged indexes, i.e. &x[n..m], &x[n..], &x[..n] and &x[..]
                if let ty::Array(_, s) = ty.sty {
                    let size: u128 = s.assert_usize(cx.tcx).unwrap().into();

                    match to_const_range(cx, range, size) {
                        (None, None) => {},
                        (Some(start), None) => {
                            if start > size {
                                utils::span_lint(
                                    cx,
                                    OUT_OF_BOUNDS_INDEXING,
                                    expr.span,
                                    "range is out of bounds",
                                );
                                return;
                            }
                        },
                        (None, Some(end)) => {
                            if end > size {
                                utils::span_lint(
                                    cx,
                                    OUT_OF_BOUNDS_INDEXING,
                                    expr.span,
                                    "range is out of bounds",
                                );
                                return;
                            }
                        },
                        (Some(start), Some(end)) => {
                            if start > size || end > size {
                                utils::span_lint(
                                    cx,
                                    OUT_OF_BOUNDS_INDEXING,
                                    expr.span,
                                    "range is out of bounds",
                                );
                            }
                            // early return because both start and end are constant
                            return;
                        },
                    }
                }

                let help_msg = match (range.start, range.end) {
                    (None, Some(_)) => "Consider using `.get(..n)`or `.get_mut(..n)` instead",
                    (Some(_), None) => "Consider using `.get(n..)` or .get_mut(n..)` instead",
                    (Some(_), Some(_)) => "Consider using `.get(n..m)` or `.get_mut(n..m)` instead",
                    (None, None) => return, // [..] is ok.
                };

                utils::span_help_and_lint(
                    cx,
                    INDEXING_SLICING,
                    expr.span,
                    "slicing may panic.",
                    help_msg,
                );
            } else {
                // Catchall non-range index, i.e. [n] or [n << m]
                if let ty::Array(..) = ty.sty {
                    // Index is a constant uint.
                    if let Some(..) = constant(cx, cx.tables, index) {
                        // Let rustc's `const_err` lint handle constant `usize` indexing on arrays.
                        return;
                    }
                }

                utils::span_help_and_lint(
                    cx,
                    INDEXING_SLICING,
                    expr.span,
                    "indexing may panic.",
                    "Consider using `.get(n)` or `.get_mut(n)` instead",
                );
            }
        }
    }
}

/// Returns a tuple of options with the start and end (exclusive) values of
/// the range. If the start or end is not constant, None is returned.
fn to_const_range<'a, 'tcx>(
    cx: &LateContext<'a, 'tcx>,
    range: Range<'_>,
    array_size: u128,
) -> (Option<u128>, Option<u128>) {
    let s = range
        .start
        .map(|expr| constant(cx, cx.tables, expr).map(|(c, _)| c));
    let start = match s {
        Some(Some(Constant::Int(x))) => Some(x),
        Some(_) => None,
        None => Some(0),
    };

    let e = range
        .end
        .map(|expr| constant(cx, cx.tables, expr).map(|(c, _)| c));
    let end = match e {
        Some(Some(Constant::Int(x))) => if range.limits == RangeLimits::Closed {
            Some(x + 1)
        } else {
            Some(x)
        },
        Some(_) => None,
        None => Some(array_size),
    };

    (start, end)
}
