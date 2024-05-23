use std::alloc;
use std::alloc::Layout;
use std::borrow::Cow;
use std::slice;

use rustc_middle::mir::interpret::AllocBytes;
use rustc_target::abi::{Align, Size};

/// Allocation bytes that explicitly handle the layout of the data they're storing.
/// This is necessary to interface with native code that accesses the program store in Miri.
#[derive(Debug)]
pub struct MiriAllocBytes {
    /// Stored layout information about the allocation.
    layout: alloc::Layout,
    /// Pointer to the allocation contents.
    /// Invariant:
    /// * If `self.layout.size() == 0`, then `self.ptr` is some suitably aligned pointer
    ///   that was allocated with the same layout but `size == 1`.
    /// * Otherwise, `self.ptr` points to memory allocated with `self.layout`.
    ptr: *mut u8,
}

impl Clone for MiriAllocBytes {
    fn clone(&self) -> Self {
        let bytes: Cow<'_, [u8]> = Cow::Borrowed(self);
        let align = Align::from_bytes(self.layout.align().try_into().unwrap()).unwrap();
        MiriAllocBytes::from_bytes(bytes, align)
    }
}

impl Drop for MiriAllocBytes {
    fn drop(&mut self) {
        let alloc_layout = if self.layout.size() == 0 {
            Layout::from_size_align(1, self.layout.align()).unwrap()
        } else {
            self.layout
        };
        // SAFETY: Invariant, `self.ptr` points to memory allocated with `self.layout`.
        unsafe { alloc::dealloc(self.ptr, alloc_layout) }
    }
}

impl std::ops::Deref for MiriAllocBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        // SAFETY: `ptr` is non-null, properly aligned, and valid for reading out `self.layout.size()`-many bytes.
        // Note that due to the invariant this is true even if `self.layout.size() == 0`.
        unsafe { slice::from_raw_parts(self.ptr, self.layout.size()) }
    }
}

impl std::ops::DerefMut for MiriAllocBytes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: `ptr` is non-null, properly aligned, and valid for reading out `self.layout.size()`-many bytes.
        // Note that due to the invariant this is true even if `self.layout.size() == 0`.
        unsafe { slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
    }
}

impl MiriAllocBytes {
    /// This method factors out how a `MiriAllocBytes` object is allocated, given a specific allocation function.
    /// If `size == 0` we allocate using a different `alloc_layout` with `size = 1`, to ensure each allocation has a unique address.
    /// Returns `Err(alloc_layout)` if the allocation function returns a `ptr` where `ptr.is_null()`.
    fn alloc_with(
        size: usize,
        align: usize,
        alloc_fn: impl FnOnce(Layout) -> *mut u8,
    ) -> Result<MiriAllocBytes, Layout> {
        let layout = Layout::from_size_align(size, align).unwrap();
        // When size is 0 we allocate 1 byte anyway, to ensure each allocation has a unique address.
        let alloc_layout =
            if size == 0 { Layout::from_size_align(1, align).unwrap() } else { layout };
        let ptr = alloc_fn(alloc_layout);
        if ptr.is_null() {
            Err(alloc_layout)
        } else {
            // SAFETY: All `MiriAllocBytes` invariants are fulfilled.
            Ok(Self { ptr, layout })
        }
    }
}

impl AllocBytes for MiriAllocBytes {
    fn from_bytes<'a>(slice: impl Into<Cow<'a, [u8]>>, align: Align) -> Self {
        let slice = slice.into();
        let size = slice.len();
        let align = align.bytes_usize();
        // SAFETY: `alloc_fn` will only be used with `size != 0`.
        let alloc_fn = |layout| unsafe { alloc::alloc(layout) };
        let alloc_bytes = MiriAllocBytes::alloc_with(size, align, alloc_fn)
            .unwrap_or_else(|layout| alloc::handle_alloc_error(layout));
        // SAFETY: `alloc_bytes.ptr` and `slice.as_ptr()` are non-null, properly aligned
        // and valid for the `size`-many bytes to be copied.
        unsafe { alloc_bytes.ptr.copy_from(slice.as_ptr(), size) };
        alloc_bytes
    }

    fn zeroed(size: Size, align: Align) -> Option<Self> {
        let size = size.bytes_usize();
        let align = align.bytes_usize();
        // SAFETY: `alloc_fn` will only be used with `size != 0`.
        let alloc_fn = |layout| unsafe { alloc::alloc_zeroed(layout) };
        MiriAllocBytes::alloc_with(size, align, alloc_fn).ok()
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }
}
