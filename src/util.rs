use std::mem::{align_of, size_of};

/// TODO: Describe what this function does and when to use it.
///
/// Converts a byte slice into a reference to a struct.
///
/// # Safety
///
/// TODO: Document safety requirements:
/// - The byte slice must be properly aligned for type T
/// - The byte slice must be exactly the size of T
/// - The bytes must represent a valid value of type T
///
/// # Panics
///
/// TODO: Document when this function panics (currently panics on misalignment or wrong size)
///
/// # Example
///
/// ```ignore
/// // TODO: Add example showing proper usage
/// // let bytes = &[...];
/// // let my_struct: &MyStruct = bytes_to_struct(bytes);
/// ```
pub(crate) fn bytes_to_struct<T: Sized>(data: &[u8]) -> &T {
    let t_len = size_of::<T>();
    assert_eq!(t_len, data.len());
    let ptr = data.as_ptr() as *const T;
    assert_eq!((ptr as usize) % align_of::<T>(), 0);
    unsafe { &*ptr }
}
