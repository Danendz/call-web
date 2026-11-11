//! Handing freed memory back to the system. The speech models are a few hundred MB of small allocations; once
//! they're dropped the allocator keeps their pages for reuse, so the app would stay that big until it quits.

/// Returns the heap's free pages to the system. Call it after dropping something large.
pub(crate) fn give_back() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: a null zone means every zone, and a goal of 0 as much as it can
        let freed = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        tracing::debug!("gave back {} MB", freed >> 20);
    }
    #[cfg(target_os = "android")]
    {
        // Bionic's mallopt option for this, from API 28
        const M_PURGE: std::ffi::c_int = -101;
        unsafe extern "C" {
            fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
        }
        // SAFETY: M_PURGE takes no pointer and only releases free pages
        unsafe { mallopt(M_PURGE, 0) };
    }
    #[cfg(target_os = "windows")]
    {
        unsafe extern "system" {
            fn GetProcessHeap() -> *mut std::ffi::c_void;
            fn HeapCompact(heap: *mut std::ffi::c_void, flags: u32) -> usize;
        }
        // SAFETY: the process heap is valid for as long as the process runs; the CRT's malloc allocates from it
        unsafe { HeapCompact(GetProcessHeap(), 0) };
    }
}
