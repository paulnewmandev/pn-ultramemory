// SPDX-License-Identifier: Apache-2.0
//! Integration tests of the report crate, run against its public API only.
//!
//! The suite renders reports from realistic, hostile and huge data and checks the results with
//! independent validators written in this test crate: an HTML and XML well-formedness checker, a
//! PDF structure checker that follows the cross-reference table, and a JSON parser. Keeping the
//! validators separate from the renderers means a bug in one cannot hide a bug in the other.

mod fixtures;
mod fuzz_tests;
mod graph_tests;
mod html_check;
mod html_tests;
mod json;
mod markdown_tests;
mod pdf_check;
mod pdf_tests;
mod scale_tests;
