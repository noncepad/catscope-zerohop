use std::mem::{align_of, size_of};

/// Converts a byte slice into a reference to a struct.
///
///
/// The caller must ensure:
/// - The byte slice must be properly aligned for type T
/// - The byte slice must be exactly the size of T
/// - The bytes must represent a valid value of type T
///
/// Panics if the slice length or alignment does not match `T`.
pub(crate) fn bytes_to_struct<T: Sized>(data: &[u8]) -> &T {
    let t_len = size_of::<T>();
    assert_eq!(t_len, data.len());
    let ptr = data.as_ptr() as *const T;
    assert_eq!((ptr as usize) % align_of::<T>(), 0);
    unsafe { &*ptr }
}
