use core::fmt::Debug;

pub trait Message: Send + Sync + Debug + 'static {}
