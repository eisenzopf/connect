//! Sample real authority time only after serialized database admission.
#[derive(Clone, Copy)]
pub(super) enum Clock {
    Current,
    #[cfg(test)]
    At(i64),
}
impl Clock {
    pub(super) fn read(self) -> i64 {
        match self {
            Self::Current => crate::now(),
            #[cfg(test)]
            Self::At(timestamp) => timestamp,
        }
    }
}
