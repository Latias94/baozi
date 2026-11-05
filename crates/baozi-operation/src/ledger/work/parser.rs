//! Parser admission errors shared by the numeric kernel and the concrete host.
use super::LedgerCause;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParserWorkError<D> {
    Ledger(LedgerCause<D>),
    ItemLimit {
        index: usize,
        value: u64,
        limit: u64,
    },
}

impl<D> From<LedgerCause<D>> for ParserWorkError<D> {
    fn from(cause: LedgerCause<D>) -> Self {
        Self::Ledger(cause)
    }
}

#[cfg(test)]
mod tests;
