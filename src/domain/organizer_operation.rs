use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OrganizerOperationId(u64);

impl OrganizerOperationId {
    pub fn allocate() -> Option<Self> {
        NEXT_OPERATION_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .ok()
            .map(Self)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for OrganizerOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::OrganizerOperationId;

    #[test]
    fn allocated_operation_ids_are_non_zero_and_unique() {
        let first = OrganizerOperationId::allocate().expect("allocate first id");
        let second = OrganizerOperationId::allocate().expect("allocate second id");

        assert_ne!(first.get(), 0);
        assert_ne!(first, second);
    }
}
