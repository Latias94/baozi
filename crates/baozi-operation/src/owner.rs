/// Marks the domain that owns an operation and its linear accounting capabilities.
///
/// Import and post-process crates define distinct zero-sized marker types and implement this trait.
/// The marker exists only at compile time and carries no runtime policy or phase meaning.
pub trait OperationOwner: Send + Sync + 'static {}
