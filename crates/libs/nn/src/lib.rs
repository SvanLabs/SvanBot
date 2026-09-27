//! A small CPU multilayer perceptron with masked softmax outputs and training.
//!
//! Part of the svanbot10 workspace; `sv10-core` re-exports every module under one path.

#![warn(missing_docs)]
// Numeric kernels index several parallel arrays by the same index.
#![allow(clippy::needless_range_loop)]

pub mod nn;
