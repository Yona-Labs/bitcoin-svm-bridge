//Utilities for working with u256 numbers represented as byte arrays [u8; 32]
pub fn add_in_place(arr: &mut [u8; 32], add: [u8; 32]) {
    let mut remainder: u16 = 0;

    for i in 0..32 {
        let pos = 31-i;
        
        let val = ((arr[pos] as u16) + (add[pos] as u16)) + remainder;
        
        let byte = val & 0xFF;
        remainder = val >> 8;

        arr[pos] = byte as u8;
    }
}

pub fn mul_in_place(arr: &mut [u8; 32], multiplicator: u32) {
    let casted_mul: u64 = multiplicator as u64;
    let mut remainder: u64 = 0;

    for i in 0..32 {
        let pos = 31-i;

        let val = ((arr[pos] as u64)*casted_mul) + remainder;

        let byte = val & 0xFF;
        remainder = val >> 8;

        arr[pos] = byte as u8;
    }
}

pub fn div_in_place(arr: &mut [u8; 32], divisor: u32) {
    let casted_div: u64 = divisor as u64;
    let mut remainder: u64 = 0;

    #[allow(clippy::needless_range_loop)]
    for i in 0..32 {
        let val: u64 = (arr[i] as u64) + remainder;
        let result = val / casted_div;

        remainder = (val % casted_div)<<8;

        arr[i] = result as u8;
    }
}

pub fn gte_arr(arr1: [u8; 32], arr2: [u8; 32]) -> bool {
    for i in 0..32 {
        if arr1[i]>arr2[i] {return true};
        if arr1[i]<arr2[i] {return false};
    }
    true
}

pub fn lte_arr(arr1: [u8; 32], arr2: [u8; 32]) -> bool {
    gte_arr(arr2, arr1)
}

pub fn gt_arr(arr1: [u8; 32], arr2: [u8; 32]) -> bool {
    for i in 0..32 {
        if arr1[i]>arr2[i] {return true};
        if arr1[i]<arr2[i] {return false};
    }
    false
}
