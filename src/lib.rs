#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use core::{
    alloc::{Layout, LayoutError},
    ptr::NonNull,
};

#[repr(C)]
pub struct HeaderSlice<T, Header = ()> {
    length: usize,
    pub header: Header,
    pub slice: [T],
}

#[repr(C)]
pub struct HeaderStr<Header = ()> {
    length: usize,
    pub header: Header,
    pub str: str,
}

pub struct HeaderSliceInitError<T, Header = ()> {
    data_ptr: *mut T,
    pub header: Header,
    length: usize,
    expected_length: usize,
}

#[cfg(feature = "alloc")]
pub enum TryNewError<Header> {
    LayoutTooLarge(Header),
    NotEnoughItems(Header),
    AllocError(Header, Layout),
}

impl<T, Header> HeaderSliceInitError<T, Header> {
    pub const fn written_len(&self) -> usize {
        self.length
    }

    pub const fn expected_length(&self) -> usize {
        self.expected_length
    }

    pub fn take_ownership(self) -> (*mut T, Header) {
        (self.data_ptr, self.header)
    }

    pub unsafe fn drop_in_place(self) -> Header {
        unsafe { core::ptr::slice_from_raw_parts_mut(self.data_ptr, self.length).drop_in_place() }
        self.header
    }
}

unsafe impl<T, Header> thin_ptr::Erasable for HeaderSlice<T, Header> {
    #[inline]
    unsafe fn unerase(this: NonNull<()>) -> NonNull<Self> {
        let ptr = this.as_ptr();
        let len = unsafe { *ptr.cast::<usize>() };
        NonNull::new_unchecked(
            core::ptr::slice_from_raw_parts_mut(ptr, len) as *mut HeaderSlice<T, Header>
        )
    }
}

unsafe impl<Header> thin_ptr::Erasable for HeaderStr<Header> {
    #[inline]
    unsafe fn unerase(this: NonNull<()>) -> core::ptr::NonNull<Self> {
        let ptr = this.as_ptr();
        let len = unsafe { *ptr.cast::<usize>() };
        NonNull::new_unchecked(
            core::ptr::slice_from_raw_parts_mut(ptr, len) as *mut HeaderStr<Header>
        )
    }
}

impl<T, Header> HeaderSlice<T, Header> {
    fn slice_head(ptr: *mut ()) -> *mut T {
        let ptr = core::ptr::slice_from_raw_parts_mut(ptr, 0) as *mut HeaderSlice<T, Header>;
        unsafe { core::ptr::addr_of_mut!((*ptr).slice).cast() }
    }

    pub fn layout_for(len: usize) -> Result<Layout, LayoutError> {
        let length = Layout::new::<usize>();
        let header = Layout::new::<Header>();
        let values = Layout::array::<T>(len)?;
        let (part1, _) = length.extend(header)?;
        let (part2, _) = part1.extend(values)?;
        Ok(part2.pad_to_align())
    }

    /// # Safety
    ///
    /// The ptr's allocation must fit `Self::layout_for(length)`
    /// ptr must be writable and unique for `Self::layout_for(length)`
    ///
    /// NOTE: if this function returns normally (i.e. doesn't panic), then you are responsible for
    /// destroying all the items written into the slice, even if this function returns `Err`!
    /// To easily do so, just call `HeaderSliceInitError::drop_in_place`
    pub unsafe fn new_into<I: IntoIterator<Item = T>>(
        ptr: NonNull<()>,
        length: usize,
        header: Header,
        iter: I,
    ) -> Result<NonNull<Self>, HeaderSliceInitError<T, Header>> {
        let data_ptr = Self::slice_head(ptr.as_ptr());
        let mut slice_writer = SliceWriter::new(data_ptr);

        for value in iter.into_iter().take(length) {
            slice_writer.write(value);
        }

        let written_len = slice_writer.len;
        slice_writer.finish();

        if written_len == length {
            let ptr =
                NonNull::new_unchecked(core::ptr::slice_from_raw_parts_mut(ptr.as_ptr(), length)
                    as *mut HeaderSlice<T, Header>);
            core::ptr::addr_of_mut!((*ptr.as_ptr()).length).write(length);
            core::ptr::addr_of_mut!((*ptr.as_ptr()).header).write(header);
            Ok(ptr)
        } else {
            Err(HeaderSliceInitError {
                data_ptr,
                header,
                length: written_len,
                expected_length: length,
            })
        }
    }

    /// # Safety
    ///
    /// The ptr's allocation must fit `Self::layout_for(length)`
    /// ptr must be writable and unique for `Self::layout_for(length)`
    pub unsafe fn clone_from_into(ptr: NonNull<()>, header: Header, slice: &[T]) -> NonNull<Self>
    where
        T: Clone,
    {
        match Self::new_into(ptr, slice.len(), header, slice.iter().cloned()) {
            Ok(x) => x,
            Err(_) => unsafe { core::hint::unreachable_unchecked() },
        }
    }

    /// # Safety
    ///
    /// The ptr's allocation must fit `Self::layout_for(length)`
    /// ptr must be writable and unique for `Self::layout_for(length)`
    pub unsafe fn copy_from_into(ptr: NonNull<()>, header: Header, slice: &[T]) -> NonNull<Self> {
        let ptr = NonNull::new_unchecked(core::ptr::slice_from_raw_parts_mut(
            ptr.as_ptr(),
            slice.len(),
        ) as *mut HeaderSlice<T, Header>);

        core::ptr::addr_of_mut!((*ptr.as_ptr()).length).write(slice.len());
        core::ptr::addr_of_mut!((*ptr.as_ptr()).header).write(header);
        let data_ptr = core::ptr::addr_of_mut!((*ptr.as_ptr()).slice).cast::<T>();
        data_ptr.copy_from_nonoverlapping(slice.as_ptr(), slice.len());

        ptr
    }

    #[cfg(feature = "alloc")]
    pub fn try_new<I: IntoIterator<Item = T>>(
        header: Header,
        iter: I,
    ) -> Result<alloc::boxed::Box<Self>, TryNewError<Header>>
    where
        I::IntoIter: ExactSizeIterator,
    {
        let iter = iter.into_iter();
        let len = iter.len();

        let ptr = match alloc::<T, Header>(len) {
            Ok(ptr) => ptr,
            Err(err) => return Err(err.with_header(header)),
        };

        match unsafe { Self::new_into(ptr.cast(), len, header, iter) } {
            Ok(ptr) => Ok(unsafe { alloc::boxed::Box::from_raw(ptr.as_ptr()) }),
            Err(err) => Err(TryNewError::NotEnoughItems(unsafe { err.drop_in_place() })),
        }
    }

    #[cfg(feature = "alloc")]
    pub fn try_clone_from(
        header: Header,
        slice: &[T],
    ) -> Result<alloc::boxed::Box<Self>, TryNewError<Header>>
    where
        T: Clone,
    {
        let ptr = match alloc::<T, Header>(slice.len()) {
            Ok(ptr) => ptr,
            Err(err) => return Err(err.with_header(header)),
        };

        let ptr = unsafe { Self::clone_from_into(ptr, header, slice) };

        Ok(unsafe { alloc::boxed::Box::from_raw(ptr.as_ptr()) })
    }

    #[cfg(feature = "alloc")]
    pub fn try_copy_from(
        header: Header,
        slice: &[T],
    ) -> Result<alloc::boxed::Box<Self>, TryNewError<Header>>
    where
        T: Copy,
    {
        let ptr = match alloc::<T, Header>(slice.len()) {
            Ok(ptr) => ptr,
            Err(err) => return Err(err.with_header(header)),
        };

        let ptr = unsafe { Self::copy_from_into(ptr, header, slice) };

        Ok(unsafe { alloc::boxed::Box::from_raw(ptr.as_ptr()) })
    }

    #[cfg(feature = "alloc")]
    pub fn new<I: IntoIterator<Item = T>>(header: Header, iter: I) -> alloc::boxed::Box<Self>
    where
        I::IntoIter: ExactSizeIterator,
    {
        match Self::try_new(header, iter) {
            Ok(x) => x,
            Err(err) => err.handle(),
        }
    }

    #[cfg(feature = "alloc")]
    pub fn clone_from<I: IntoIterator<Item = T>>(
        header: Header,
        iter: &[T],
    ) -> alloc::boxed::Box<Self>
    where
        T: Clone,
    {
        match Self::try_clone_from(header, iter) {
            Ok(x) => x,
            Err(err) => err.handle(),
        }
    }

    #[cfg(feature = "alloc")]
    pub fn copy_from(header: Header, iter: &[T]) -> alloc::boxed::Box<Self>
    where
        T: Copy,
    {
        match Self::try_copy_from(header, iter) {
            Ok(x) => x,
            Err(err) => err.handle(),
        }
    }
}

#[cfg(feature = "alloc")]
impl TryNewError<()> {
    fn with_header<Header>(self, header: Header) -> TryNewError<Header> {
        match self {
            TryNewError::LayoutTooLarge(()) => TryNewError::LayoutTooLarge(header),
            TryNewError::NotEnoughItems(()) => TryNewError::NotEnoughItems(header),
            TryNewError::AllocError((), layout) => TryNewError::AllocError(header, layout),
        }
    }
}

#[cfg(feature = "alloc")]
impl<T> TryNewError<T> {
    #[cold]
    #[inline(never)]
    fn handle(self) -> ! {
        match self {
            TryNewError::LayoutTooLarge(_) => {
                fn layout_too_large() -> ! {
                    panic!("length too large to allocate")
                }

                layout_too_large()
            }
            TryNewError::AllocError(_, layout) => alloc::alloc::handle_alloc_error(layout),
            TryNewError::NotEnoughItems(_) => {
                fn new_failed() -> ! {
                    panic!("Not enough items provided in iterator")
                }

                new_failed()
            }
        }
    }
}

#[cfg(feature = "alloc")]
fn alloc<T, Header>(len: usize) -> Result<NonNull<()>, TryNewError<()>> {
    let layout = match HeaderSlice::<T, Header>::layout_for(len) {
        Ok(layout) => layout,
        Err(_) => return Err(TryNewError::LayoutTooLarge(())),
    };

    let Some(ptr) = NonNull::new(unsafe { alloc::alloc::alloc(layout) }) else {
        return Err(TryNewError::AllocError((), layout));
    };

    Ok(ptr.cast())
}

impl<Header> HeaderStr<Header> {
    pub fn layout_for(len: usize) -> Result<Layout, LayoutError> {
        let length = Layout::new::<usize>();
        let header = Layout::new::<Header>();
        let values = Layout::array::<u8>(len)?;
        let (part1, _) = length.extend(header)?;
        let (part2, _) = part1.extend(values)?;
        Ok(part2.pad_to_align())
    }

    fn cast(ptr: NonNull<HeaderSlice<u8, Header>>) -> NonNull<HeaderStr<Header>> {
        unsafe { NonNull::new_unchecked(ptr.as_ptr() as *mut HeaderStr<Header>) }
    }

    /// # Safety
    ///
    /// The ptr's allocation must fit `Self::layout_for(length)`
    /// ptr must be writable and unique for `Self::layout_for(length)`
    pub unsafe fn new_into(ptr: NonNull<()>, s: &str, header: Header) -> NonNull<Self> {
        Self::cast(HeaderSlice::<u8, Header>::copy_from_into(
            ptr,
            header,
            s.as_bytes(),
        ))
    }
}

struct SliceWriter<T> {
    start: *mut T,
    ptr: *mut T,
    len: usize,
}

impl<T> SliceWriter<T> {
    unsafe fn new(ptr: *mut T) -> Self {
        Self {
            start: ptr,
            ptr,
            len: 0,
        }
    }

    unsafe fn write(&mut self, value: T) {
        self.ptr.write(value);
        self.len += 1;
    }

    fn finish(self) {
        core::mem::forget(self);
    }
}

impl<T> Drop for SliceWriter<T> {
    fn drop(&mut self) {
        unsafe { core::ptr::slice_from_raw_parts_mut(self.start, self.len).drop_in_place() }
    }
}

impl<Header: PartialEq, T: PartialEq> PartialEq for HeaderSlice<T, Header> {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header && self.slice == other.slice
    }
}

impl<Header: Eq, T: Eq> Eq for HeaderSlice<T, Header> {}

impl<Header: PartialOrd, T: PartialOrd> PartialOrd for HeaderSlice<T, Header> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match self.header.partial_cmp(&other.header)? {
            o @ (core::cmp::Ordering::Less | core::cmp::Ordering::Greater) => Some(o),
            core::cmp::Ordering::Equal => self.slice.partial_cmp(&other.slice),
        }
    }
}

impl<Header: Ord, T: Ord> Ord for HeaderSlice<T, Header> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match self.header.cmp(&other.header) {
            o @ (core::cmp::Ordering::Less | core::cmp::Ordering::Greater) => o,
            core::cmp::Ordering::Equal => self.slice.cmp(&other.slice),
        }
    }
}

impl<T: core::hash::Hash, Header: core::hash::Hash> core::hash::Hash for HeaderSlice<T, Header> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.header.hash(state);
        self.slice.hash(state);
    }
}

impl<Header: PartialEq> PartialEq for HeaderStr<Header> {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header && self.str == other.str
    }
}

impl<Header: Eq> Eq for HeaderStr<Header> {}

impl<Header: PartialOrd> PartialOrd for HeaderStr<Header> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match self.header.partial_cmp(&other.header)? {
            o @ (core::cmp::Ordering::Less | core::cmp::Ordering::Greater) => Some(o),
            core::cmp::Ordering::Equal => self.str.partial_cmp(&other.str),
        }
    }
}

impl<Header: Ord> Ord for HeaderStr<Header> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match self.header.cmp(&other.header) {
            o @ (core::cmp::Ordering::Less | core::cmp::Ordering::Greater) => o,
            core::cmp::Ordering::Equal => self.str.cmp(&other.str),
        }
    }
}

impl<Header: core::hash::Hash> core::hash::Hash for HeaderStr<Header> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.header.hash(state);
        self.str.hash(state);
    }
}
