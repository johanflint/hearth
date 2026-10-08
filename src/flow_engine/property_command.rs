use crate::flow_engine::{Expression, Value};
use serde::Deserialize;
use std::time::Duration;

#[derive(PartialEq, Debug)]
pub struct PropertyCommand {
    pub operation: Operation,
    pub value: Expression,
    pub transition: Option<Duration>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ResolvedPropertyCommand {
    pub operation: Operation,
    pub value: Value,
    pub transition: Option<Duration>,
}

#[derive(Clone, PartialEq, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Operation { Set, Toggle, Increment, Decrement }

impl Operation {
    pub(crate) fn conflict_merge_semantics(&self) -> ConflictMergeSemantics {
        match self {
            Operation::Set => ConflictMergeSemantics::DeduplicateIfEqual,
            Operation::Toggle | Operation::Increment | Operation::Decrement => ConflictMergeSemantics::Conflict,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum ConflictMergeSemantics {
    /// Equal proposals from different flows can be collapsed into a single dispatch
    DeduplicateIfEqual,
    /// If two [PropertyCommand]s are semantically distinct, do not mean agreement (e.g. two toggles or increments are
    /// not the same as applying one
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::set(Operation::Set, ConflictMergeSemantics::DeduplicateIfEqual)]
    #[case::toggle(Operation::Toggle, ConflictMergeSemantics::Conflict)]
    #[case::increment(Operation::Increment, ConflictMergeSemantics::Conflict)]
    #[case::decrement(Operation::Decrement, ConflictMergeSemantics::Conflict)]
    fn conflict_merge_semantics_maps_each_operation(#[case] operation: Operation, #[case] expected: ConflictMergeSemantics) {
        assert_eq!(operation.conflict_merge_semantics(), expected);
    }
}
