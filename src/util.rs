use std::mem::{align_of, size_of};

use solana_sdk::pubkey::Pubkey;

use crate::err::CatscopeZerohopError;

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

pub fn as_bytes_mut<T: Sized>(val: &mut T) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut((val as *mut T) as *mut u8, std::mem::size_of::<T>()) }
}

pub fn as_bytes<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((val as *const T) as *const u8, std::mem::size_of::<T>()) }
}

pub fn as_bytes_slice<T: Sized>(s: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * std::mem::size_of::<T>())
    }
}

pub struct TokenReference<'a> {
    pub owner: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub amount: u64,
}

//TODO: add in the rest of the token parameters like delegate account
pub fn parse_token<'a, 'b: 'a>(data: &'b [u8]) -> Result<TokenReference<'a>, CatscopeZerohopError> {
    let p_len = std::mem::size_of::<Pubkey>();
    let amt_len = std::mem::size_of::<u64>();
    let mut index = 0;
    if index + p_len < data.len() {
        return Err(CatscopeZerohopError::OutofRange);
    }
    let mint = unsafe {
        let subbuf = &data[index..(index + p_len)];
        index += p_len;
        let ptr = subbuf.as_ptr() as *const Pubkey;
        &*ptr
    };
    if index + p_len < data.len() {
        return Err(CatscopeZerohopError::OutofRange);
    }
    let owner = unsafe {
        let subbuf = &data[index..(index + p_len)];
        index += p_len;
        let ptr = subbuf.as_ptr() as *const Pubkey;
        &*ptr
    };
    if index + p_len < data.len() {
        return Err(CatscopeZerohopError::OutofRange);
    }
    let amount = {
        let s = &data[index..(index + amt_len)];
        //        index += amt_len;
        let subbuf: [u8; 8] = s.try_into().unwrap();
        u64::from_le_bytes(subbuf)
    };
    Ok(TokenReference {
        mint,
        owner,
        amount,
    })
}
