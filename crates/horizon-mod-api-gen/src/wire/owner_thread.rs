/// A native handler whose implementation need not be Send. Only its creating thread may
/// inspect it. A foreign-thread drop intentionally leaks the value rather than running a
/// potentially thread-affine destructor there.
pub struct OwnerThread<T> {
    owner: std::thread::ThreadId,
    value: std::mem::ManuallyDrop<T>,
}
// SAFETY: no reference to T is obtainable on a foreign thread; Drop never touches T there.
unsafe impl<T> Send for OwnerThread<T> {}
unsafe impl<T> Sync for OwnerThread<T> {}
impl<T> OwnerThread<T> {
    pub fn new(value: T) -> Self {
        Self {
            owner: std::thread::current().id(),
            value: std::mem::ManuallyDrop::new(value),
        }
    }
    pub fn get(&self) -> Result<&T, ServeError> {
        if self.owner != std::thread::current().id() {
            return Err(ServeError::WrongThread);
        }
        Ok(&self.value)
    }
}
impl<T> Drop for OwnerThread<T> {
    fn drop(&mut self) {
        if self.owner == std::thread::current().id() {
            // SAFETY: this is the creating thread and this is the only manual drop.
            unsafe { std::mem::ManuallyDrop::drop(&mut self.value) };
        }
    }
}
