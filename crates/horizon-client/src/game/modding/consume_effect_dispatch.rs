//! Call-stack recursion boundary shared by ordinary and explicit consume effects.
use std::cell::RefCell;

thread_local! {
    static ACTIVE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct ActiveKind;
impl Drop for ActiveKind {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().pop();
        });
    }
}

/// Returns false only when this kind already has a handler on this call stack.
/// Different kinds may dispatch normally, including kinds owned by another mod.
pub(super) fn dispatch(kind: &str, apply: impl FnOnce()) -> bool {
    let entered = ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        if active.iter().any(|current| current == kind) {
            return false;
        }
        active.push(kind.to_owned());
        true
    });
    if !entered {
        return false;
    }
    let _guard = ActiveKind;
    apply();
    true
}

#[cfg(test)]
mod tests {
    use super::dispatch;
    #[test]
    fn nested_same_kind_is_refused_but_another_kind_and_later_calls_run() {
        let mut calls = Vec::new();
        assert!(dispatch("first:effect", || {
            calls.push(1);
            assert!(!dispatch("first:effect", || panic!("same kind recursed")));
            assert!(dispatch("second:effect", || {
                calls.push(2);
                assert!(!dispatch("first:effect", || panic!(
                    "indirect cycle recursed"
                )));
            }));
        }));
        assert!(dispatch("first:effect", || calls.push(3)));
        assert_eq!(calls, [1, 2, 3]);
    }
    #[test]
    fn an_unwinding_handler_does_not_leave_the_kind_active() {
        assert!(
            std::panic::catch_unwind(|| dispatch("panic:effect", || panic!("fixture"))).is_err()
        );
        let mut ran = false;
        assert!(dispatch("panic:effect", || ran = true));
        assert!(ran);
    }
}
