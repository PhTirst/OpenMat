//! Official-HDF5 implementation of the narrow MAT 7.3 provider boundary.
//!
//! Unsafe code is confined to the file-image and global plugin-loading calls
//! that `hdf5-metno` does not currently expose as safe high-level operations.

#![allow(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    ffi::{CString, c_uint, c_void},
    mem::size_of,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use hdf5::{
    Dataset, Dataspace, Datatype, File, Group, H5Type, Hyperslab, LinkType, Location, LocationType,
    ObjectReference1, ReferencedObject, SliceOrIndex,
    file::{FileAccess, LibraryVersion},
    filters::Filter,
    plist::dataset_create::Layout,
    types::{FixedAscii, VarLenArray, VarLenAscii},
};
use hdf5_metno as hdf5;
use hdf5_metno_sys::{
    h5d::{H5Dread, H5Dwrite},
    h5f::{H5F_ACC_RDONLY, H5F_ACC_RDWR, H5Fget_file_image, H5Fopen},
    h5p::{H5P_DEFAULT, H5Pset_file_image},
};
use openmat_array::{
    ArrayData, CharCodeUnit, Complex32, Complex64, ComplexInteger, DenseArray, IntegerArrayData,
    Logical, Shape,
};
use openmat_value::{CellArray, CscMatrix, FieldName, SparseArrayData, StructArray, Value};

use super::{
    Hdf5Provider, Mat73Backend, Mat73Hyperslab, MatError, MatErrorKind, MatLimits, MatVariable,
    validate_name,
};

const MATLAB_HEADER_BYTES: usize = 128;
const MATLAB_USER_BLOCK_BYTES: usize = 512;
const HDF5_SIGNATURE: &[u8; 8] = b"\x89HDF\r\n\x1a\n";
const COMPRESSION_THRESHOLD_BYTES: usize = 1_024;
const CANCELLATION_CHECK_INTERVAL: usize = 4_096;
const MAX_NAME_BYTES: usize = 4_096;

static IMAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static PLUGINS_DISABLED: OnceLock<Result<(), String>> = OnceLock::new();

// hdf5-metno-sys 0.12.3 declares this HDF5 2.2 API with a pointer argument;
// the bundled official header and implementation use `unsigned int`.
unsafe extern "C" {
    #[link_name = "H5PLset_loading_state"]
    fn h5pl_set_loading_state(plugin_control_mask: c_uint) -> i32;
}

/// Returns whether the bytes carry the MATLAB 7.3 user-block header and HDF5 signature.
pub(super) fn is_mat73(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MATLAB 7.3 MAT-file")
        && bytes.get(MATLAB_USER_BLOCK_BYTES..MATLAB_USER_BLOCK_BYTES + HDF5_SIGNATURE.len())
            == Some(HDF5_SIGNATURE)
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct OfficialHdf5Provider;

pub(super) fn default_backend() -> Mat73Backend<OfficialHdf5Provider> {
    Mat73Backend::new(OfficialHdf5Provider)
}

impl Hdf5Provider for OfficialHdf5Provider {
    fn decode(
        &self,
        bytes: &[u8],
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<MatVariable>, MatError> {
        Decoder::new(bytes, limits, cancellation)?.decode()
    }

    fn encode(
        &self,
        variables: &[MatVariable],
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, MatError> {
        Encoder::new(limits, cancellation)?.encode(variables)
    }

    fn read_dense_hyperslab(
        &self,
        bytes: &[u8],
        variable: &str,
        selection: &Mat73Hyperslab,
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Value, MatError> {
        Decoder::new(bytes, limits, cancellation)?.read_dense_hyperslab(variable, selection)
    }

    fn write_dense_hyperslab(
        &self,
        bytes: &[u8],
        variable: &str,
        selection: &Mat73Hyperslab,
        value: &Value,
        limits: MatLimits,
        cancellation: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, MatError> {
        Decoder::new_writable(bytes, limits, cancellation)?
            .write_dense_hyperslab(variable, selection, value)
    }
}

fn invalid(message: impl Into<String>) -> MatError {
    MatError::with_kind(MatErrorKind::InvalidFormat, message)
}

fn unsupported(message: impl Into<String>) -> MatError {
    MatError::with_kind(MatErrorKind::Unsupported, message)
}

fn limit(message: impl Into<String>) -> MatError {
    MatError::with_kind(MatErrorKind::LimitExceeded, message)
}

fn cancelled() -> MatError {
    MatError::with_kind(MatErrorKind::Cancelled, "MAT-file operation was cancelled")
}

fn hdf5_error(context: &str, error: impl std::fmt::Display) -> MatError {
    MatError::with_kind(MatErrorKind::Hdf5, format!("{context}: {error}"))
}

fn invalid_value(message: impl Into<String>) -> MatError {
    MatError::with_kind(MatErrorKind::InvalidValue, message)
}

fn check_cancelled(cancellation: Option<&AtomicBool>) -> Result<(), MatError> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

fn disable_dynamic_plugins() -> Result<(), MatError> {
    let result = PLUGINS_DISABLED.get_or_init(|| {
        hdf5::sync::sync(|| {
            // SAFETY: the bundled official HDF5 header declares this function with
            // one unsigned mask; zero disables every dynamically loaded plugin type.
            let status = unsafe { h5pl_set_loading_state(0) };
            if status < 0 {
                Err(String::from(
                    "official HDF5 library refused to disable dynamic plugin loading",
                ))
            } else {
                Ok(())
            }
        })
    });
    result
        .clone()
        .map_err(|message| hdf5_error("cannot secure HDF5 plugin state", message))
}

fn file_access() -> Result<FileAccess, MatError> {
    let mut builder = FileAccess::build();
    builder
        .core_filebacked(false)
        .elink_file_cache_size(0)
        .chunk_cache(521, 8 * 1024 * 1024, 0.75)
        .libver_bounds(LibraryVersion::Earliest, LibraryVersion::V110);
    builder
        .finish()
        .map_err(|error| hdf5_error("cannot create bounded HDF5 file access properties", error))
}

struct ImageFile {
    file: File,
    _source_image: Vec<u8>,
}

impl ImageFile {
    fn open(bytes: &[u8]) -> Result<Self, MatError> {
        Self::open_with_access(bytes, H5F_ACC_RDONLY)
    }

    fn open_writable(bytes: &[u8]) -> Result<Self, MatError> {
        Self::open_with_access(bytes, H5F_ACC_RDWR)
    }

    fn open_with_access(bytes: &[u8], access: u32) -> Result<Self, MatError> {
        disable_dynamic_plugins()?;
        let mut source_image = Vec::new();
        source_image
            .try_reserve_exact(bytes.len())
            .map_err(|_| limit("MAT v7.3 file image allocation failed"))?;
        source_image.extend_from_slice(bytes);
        let fapl = file_access()?;
        let sequence = IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let filename = CString::new(format!("openmat-mat73-read-image-{sequence}"))
            .expect("generated HDF5 image name contains no NUL");
        let file = hdf5::sync::sync(|| {
            // SAFETY: the mutable image buffer remains alive in `ImageFile`; HDF5
            // receives its exact length, and the returned identifier is transferred
            // once into the high-level owner on success.
            let status = unsafe {
                H5Pset_file_image(
                    fapl.id(),
                    source_image.as_mut_ptr().cast::<c_void>(),
                    source_image.len(),
                )
            };
            if status < 0 {
                return Err(hdf5_error(
                    "cannot attach the HDF5 file image",
                    "H5Pset_file_image failed",
                ));
            }
            // SAFETY: `filename` and `fapl` remain valid for the call. The core
            // driver opens the supplied image read-only and does not access disk.
            let id = unsafe { H5Fopen(filename.as_ptr(), access, fapl.id()) };
            if id < 0 {
                return Err(hdf5_error(
                    "cannot open the HDF5 file image",
                    "H5Fopen failed",
                ));
            }
            // SAFETY: `id` is a fresh HDF5 file identifier with reference count 1.
            unsafe { hdf5::from_id::<File>(id) }
                .map_err(|error| hdf5_error("cannot own the HDF5 file image", error))
        })?;
        Ok(Self {
            file,
            _source_image: source_image,
        })
    }
}

fn extract_image(
    file: &File,
    maximum_file_bytes: usize,
    maximum_allocation_bytes: usize,
) -> Result<Vec<u8>, MatError> {
    file.flush()
        .map_err(|error| hdf5_error("cannot flush MAT v7.3 image", error))?;
    let length = hdf5::sync::sync(|| {
        // SAFETY: a null destination with length zero queries the required size.
        unsafe { H5Fget_file_image(file.id(), std::ptr::null_mut(), 0) }
    });
    let length = usize::try_from(length)
        .map_err(|_| hdf5_error("cannot size MAT v7.3 image", "H5Fget_file_image failed"))?;
    let output_length = length
        .checked_add(MATLAB_USER_BLOCK_BYTES)
        .ok_or_else(|| limit("MAT v7.3 output length overflow"))?;
    if output_length > maximum_file_bytes {
        return Err(limit(format!(
            "MAT v7.3 output is {output_length} bytes, exceeding the configured {maximum_file_bytes}-byte file limit"
        )));
    }
    if output_length > maximum_allocation_bytes {
        return Err(limit(format!(
            "MAT v7.3 output image is {output_length} bytes, exceeding the configured {maximum_allocation_bytes}-byte allocation limit"
        )));
    }
    let mut image = Vec::new();
    image
        .try_reserve_exact(output_length)
        .map_err(|_| limit("MAT v7.3 output image allocation failed"))?;
    image.resize(output_length, 0);
    let copied = hdf5::sync::sync(|| {
        // SAFETY: the slice after the reserved user block is writable for
        // exactly `length` bytes.
        unsafe {
            H5Fget_file_image(
                file.id(),
                image
                    .as_mut_ptr()
                    .add(MATLAB_USER_BLOCK_BYTES)
                    .cast::<c_void>(),
                length,
            )
        }
    });
    if copied < 0 || usize::try_from(copied).ok() != Some(length) {
        return Err(hdf5_error(
            "cannot copy MAT v7.3 image",
            "H5Fget_file_image returned an inconsistent length",
        ));
    }
    if image.len() < MATLAB_USER_BLOCK_BYTES + HDF5_SIGNATURE.len()
        || image[MATLAB_USER_BLOCK_BYTES..MATLAB_USER_BLOCK_BYTES + HDF5_SIGNATURE.len()]
            != *HDF5_SIGNATURE
    {
        return Err(hdf5_error(
            "cannot finalize MAT v7.3 image",
            "official HDF5 output is missing the expected 512-byte user block",
        ));
    }
    write_matlab_header(&mut image)?;
    Ok(image)
}

fn write_matlab_header(image: &mut [u8]) -> Result<(), MatError> {
    let header = image
        .get_mut(..MATLAB_HEADER_BYTES)
        .ok_or_else(|| invalid("MAT v7.3 output is shorter than its user-block header"))?;
    header.fill(b' ');
    let description =
        b"MATLAB 7.3 MAT-file, Platform: OpenMat, Created by OpenMat, HDF5 schema 1.00 .";
    header[..description.len()].copy_from_slice(description);
    header[116..124].fill(0);
    header[124] = 0;
    header[125] = 2;
    header[126] = b'I';
    header[127] = b'M';
    Ok(())
}

#[derive(H5Type, Clone, Copy)]
#[repr(C)]
struct CompoundF32 {
    real: f32,
    imag: f32,
}

#[derive(H5Type, Clone, Copy)]
#[repr(C)]
struct CompoundF64 {
    real: f64,
    imag: f64,
}

macro_rules! compound_integer_type {
    ($name:ident, $ty:ty) => {
        #[derive(H5Type, Clone, Copy)]
        #[repr(C)]
        struct $name {
            real: $ty,
            imag: $ty,
        }
    };
}

compound_integer_type!(CompoundI8, i8);
compound_integer_type!(CompoundU8, u8);
compound_integer_type!(CompoundI16, i16);
compound_integer_type!(CompoundU16, u16);
compound_integer_type!(CompoundI32, i32);
compound_integer_type!(CompoundU32, u32);
compound_integer_type!(CompoundI64, i64);
compound_integer_type!(CompoundU64, u64);

enum LinkBatchFailure {
    Allocation,
    AllocationLimit,
    Cancelled,
    NameTooLong(usize),
    ObjectLimit,
}

#[derive(Default)]
struct LinkBatch {
    entries: Vec<(String, LinkType)>,
    allocation_bytes: usize,
    failure: Option<LinkBatchFailure>,
}

#[allow(clippy::default_trait_access)]
fn collect_group_links(
    group: &Group,
    maximum_entries: usize,
    maximum_allocation: usize,
    cancellation: Option<&AtomicBool>,
) -> Result<LinkBatch, MatError> {
    let batch = group
        .iter_visit(
            Default::default(),
            Default::default(),
            LinkBatch::default(),
            |_, name, info, batch| {
                if batch
                    .entries
                    .len()
                    .is_multiple_of(CANCELLATION_CHECK_INTERVAL)
                    && cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
                {
                    batch.failure = Some(LinkBatchFailure::Cancelled);
                    return false;
                }
                if batch.entries.len() >= maximum_entries {
                    batch.failure = Some(LinkBatchFailure::ObjectLimit);
                    return false;
                }
                if name.len() > MAX_NAME_BYTES {
                    batch.failure = Some(LinkBatchFailure::NameTooLong(name.len()));
                    return false;
                }
                let Some(allocation_bytes) = name
                    .len()
                    .checked_add(size_of::<(String, LinkType)>())
                    .and_then(|bytes| batch.allocation_bytes.checked_add(bytes))
                else {
                    batch.failure = Some(LinkBatchFailure::AllocationLimit);
                    return false;
                };
                if allocation_bytes > maximum_allocation {
                    batch.failure = Some(LinkBatchFailure::AllocationLimit);
                    return false;
                }
                if batch.entries.try_reserve(1).is_err() {
                    batch.failure = Some(LinkBatchFailure::Allocation);
                    return false;
                }
                let mut owned_name = String::new();
                if owned_name.try_reserve_exact(name.len()).is_err() {
                    batch.failure = Some(LinkBatchFailure::Allocation);
                    return false;
                }
                owned_name.push_str(name);
                batch.entries.push((owned_name, info.link_type));
                batch.allocation_bytes = allocation_bytes;
                true
            },
        )
        .map_err(|error| hdf5_error("cannot enumerate MAT v7.3 HDF5 links", error))?;
    match batch.failure {
        Some(LinkBatchFailure::Allocation) => Err(limit("MAT v7.3 link table allocation failed")),
        Some(LinkBatchFailure::AllocationLimit) => Err(limit(
            "MAT v7.3 link table exceeds its configured allocation limit",
        )),
        Some(LinkBatchFailure::Cancelled) => Err(cancelled()),
        Some(LinkBatchFailure::NameTooLong(length)) => Err(limit(format!(
            "HDF5 object name is {length} bytes, exceeding the {MAX_NAME_BYTES}-byte limit"
        ))),
        Some(LinkBatchFailure::ObjectLimit) => {
            Err(limit("MAT v7.3 object count exceeds its configured limit"))
        }
        None => Ok(batch),
    }
}

struct Decoder<'a> {
    image: ImageFile,
    limits: MatLimits,
    cancellation: Option<&'a AtomicBool>,
    allocated_bytes: usize,
    decompressed_bytes: usize,
    reference_count: usize,
    active_references: HashSet<String>,
    cached_references: HashMap<String, Value>,
}

impl<'a> Decoder<'a> {
    fn new(
        bytes: &[u8],
        limits: MatLimits,
        cancellation: Option<&'a AtomicBool>,
    ) -> Result<Self, MatError> {
        check_cancelled(cancellation)?;
        if !is_mat73(bytes) {
            return Err(invalid(
                "input is not a MATLAB 7.3 MAT-file with a 512-byte HDF5 user block",
            ));
        }
        if bytes.len() > limits.maximum_file_bytes {
            return Err(limit(format!(
                "MAT v7.3 input is {} bytes, exceeding the configured {}-byte file limit",
                bytes.len(),
                limits.maximum_file_bytes
            )));
        }
        if bytes.len() > limits.maximum_allocation_bytes {
            return Err(limit(format!(
                "MAT v7.3 file image is {} bytes, exceeding the configured {}-byte allocation limit",
                bytes.len(),
                limits.maximum_allocation_bytes
            )));
        }
        let image = ImageFile::open(bytes)?;
        Ok(Self {
            image,
            limits,
            cancellation,
            allocated_bytes: bytes.len(),
            decompressed_bytes: 0,
            reference_count: 0,
            active_references: HashSet::new(),
            cached_references: HashMap::new(),
        })
    }

    fn new_writable(
        bytes: &[u8],
        limits: MatLimits,
        cancellation: Option<&'a AtomicBool>,
    ) -> Result<Self, MatError> {
        let mut decoder = Self::new(bytes, limits, cancellation)?;
        decoder.image = ImageFile::open_writable(bytes)?;
        Ok(decoder)
    }

    #[allow(clippy::too_many_lines)]
    fn read_dense_hyperslab(
        mut self,
        variable: &str,
        selection: &Mat73Hyperslab,
    ) -> Result<Value, MatError> {
        self.validate_hdf5_surface()?;
        let dataset = self.open_dense_variable(variable)?;
        let (shape, hyperslab, expected) = self.validate_hyperslab(&dataset, selection)?;
        let class = read_class(&dataset)?;
        match class.as_str() {
            "double" => {
                if dataset
                    .dtype()
                    .map_err(|error| hdf5_error("cannot inspect double type", error))?
                    .is::<CompoundF64>()
                {
                    let raw = self.read_hyperslab_raw::<CompoundF64>(
                        &dataset,
                        hyperslab,
                        expected,
                        "complex double hyperslab",
                    )?;
                    let values = self.convert_values(raw, "complex double hyperslab", |value| {
                        Complex64::new(value.real, value.imag)
                    })?;
                    dense_complex_value(shape, values)
                } else {
                    ensure_dataset_type::<f64>(&dataset, "double")?;
                    let values = self.read_hyperslab_raw::<f64>(
                        &dataset,
                        hyperslab,
                        expected,
                        "double hyperslab",
                    )?;
                    dense_f64_value(shape, values)
                }
            }
            "single" => {
                if dataset
                    .dtype()
                    .map_err(|error| hdf5_error("cannot inspect single type", error))?
                    .is::<CompoundF32>()
                {
                    let raw = self.read_hyperslab_raw::<CompoundF32>(
                        &dataset,
                        hyperslab,
                        expected,
                        "complex single hyperslab",
                    )?;
                    let values = self.convert_values(raw, "complex single hyperslab", |value| {
                        Complex32::new(value.real, value.imag)
                    })?;
                    DenseArray::from_vec(shape, values)
                        .map(ArrayData::ComplexF32)
                        .map(Value::Array)
                        .map_err(array_error)
                } else {
                    ensure_dataset_type::<f32>(&dataset, "single")?;
                    let values = self.read_hyperslab_raw::<f32>(
                        &dataset,
                        hyperslab,
                        expected,
                        "single hyperslab",
                    )?;
                    DenseArray::from_vec(shape, values)
                        .map(ArrayData::F32)
                        .map(Value::Array)
                        .map_err(array_error)
                }
            }
            "logical" => {
                ensure_decode_attr(&dataset, 1)?;
                ensure_dataset_type::<u8>(&dataset, "logical")?;
                let raw = self.read_hyperslab_raw::<u8>(
                    &dataset,
                    hyperslab,
                    expected,
                    "logical hyperslab",
                )?;
                let values = self
                    .convert_values(raw, "logical hyperslab", |value| Logical::from(value != 0))?;
                dense_logical_value(shape, values)
            }
            "char" => {
                ensure_decode_attr(&dataset, 2)?;
                ensure_dataset_type::<u16>(&dataset, "char")?;
                let raw = self.read_hyperslab_raw::<u16>(
                    &dataset,
                    hyperslab,
                    expected,
                    "char hyperslab",
                )?;
                let values = self.convert_values(raw, "char hyperslab", CharCodeUnit::new)?;
                DenseArray::from_vec(shape, values)
                    .map(ArrayData::Char)
                    .map(Value::Array)
                    .map_err(array_error)
            }
            "int8" => self.read_integer_hyperslab::<i8>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            "uint8" => self.read_integer_hyperslab::<u8>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            "int16" => self.read_integer_hyperslab::<i16>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            "uint16" => self.read_integer_hyperslab::<u16>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            "int32" => self.read_integer_hyperslab::<i32>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            "uint32" => self.read_integer_hyperslab::<u32>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            "int64" => self.read_integer_hyperslab::<i64>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            "uint64" => self.read_integer_hyperslab::<u64>(
                &dataset,
                hyperslab,
                shape,
                expected,
                IntegerArrayData::from_typed,
            ),
            other => Err(unsupported(format!(
                "MAT variable '{variable}' of class '{other}' does not support dense hyperslab reads"
            ))),
        }
    }

    fn write_dense_hyperslab(
        mut self,
        variable: &str,
        selection: &Mat73Hyperslab,
        value: &Value,
    ) -> Result<Vec<u8>, MatError> {
        self.validate_hdf5_surface()?;
        let dataset = self.open_dense_variable(variable)?;
        let (shape, hyperslab, expected) = self.validate_hyperslab(&dataset, selection)?;
        if value.dimensions() != Some(shape.dimensions()) {
            return Err(invalid_value(format!(
                "replacement value shape {:?} does not match hyperslab shape {:?}",
                value.dimensions(),
                shape.dimensions()
            )));
        }
        let class = read_class(&dataset)?;
        self.write_hyperslab_value(&dataset, hyperslab, expected, &class, value)?;
        check_cancelled(self.cancellation)?;
        extract_image(
            &self.image.file,
            self.limits.maximum_file_bytes,
            self.limits.maximum_allocation_bytes,
        )
    }

    fn open_dense_variable(&self, variable: &str) -> Result<Dataset, MatError> {
        validate_hdf5_name(variable, "MAT variable")?;
        let root = self
            .image
            .file
            .group("/")
            .map_err(|error| hdf5_error("cannot open MAT v7.3 root group", error))?;
        if root.loc_type_by_name(variable).map_err(|error| {
            invalid(format!("MAT variable '{variable}' does not exist: {error}"))
        })? != LocationType::Dataset
        {
            return Err(unsupported(format!(
                "MAT variable '{variable}' is not a dense dataset"
            )));
        }
        let dataset = root.dataset(variable).map_err(|error| {
            hdf5_error(&format!("cannot open MAT variable '{variable}'"), error)
        })?;
        if read_optional_u8_attr(&dataset, "MATLAB_empty")?.is_some() {
            return Err(unsupported(format!(
                "MAT variable '{variable}' uses shaped-empty metadata rather than dense storage"
            )));
        }
        Ok(dataset)
    }

    fn validate_hyperslab(
        &self,
        dataset: &Dataset,
        selection: &Mat73Hyperslab,
    ) -> Result<(Shape, Hyperslab, usize), MatError> {
        let dataset_shape = dataset_shape(dataset)?;
        if selection.start().len() != dataset_shape.ndims() {
            return Err(invalid_value(format!(
                "MAT hyperslab has {} dimensions for a {}-dimensional variable",
                selection.start().len(),
                dataset_shape.ndims()
            )));
        }
        for (dimension, ((start, count), extent)) in selection
            .start()
            .iter()
            .zip(selection.count())
            .zip(dataset_shape.dimensions())
            .enumerate()
        {
            let end = start.checked_add(*count).ok_or_else(|| {
                invalid_value(format!("MAT hyperslab dimension {dimension} overflows"))
            })?;
            if end > *extent {
                return Err(invalid_value(format!(
                    "MAT hyperslab dimension {dimension} ends at {end}, beyond extent {extent}"
                )));
            }
        }
        let shape = Shape::new(selection.count().iter().copied())
            .map_err(|error| invalid_value(format!("invalid MAT hyperslab shape: {error}")))?;
        let expected = self.check_shape(&shape)?;
        let mut slices = Vec::new();
        slices
            .try_reserve_exact(selection.start().len())
            .map_err(|_| limit("MAT hyperslab selection allocation failed"))?;
        for (start, count) in selection.start().iter().zip(selection.count()).rev() {
            slices.push(SliceOrIndex::SliceCount {
                start: usize::try_from(*start)
                    .map_err(|_| limit("MAT hyperslab start is not host-representable"))?,
                step: 1,
                count: usize::try_from(*count)
                    .map_err(|_| limit("MAT hyperslab count is not host-representable"))?,
                block: 1,
            });
        }
        Ok((shape, Hyperslab::from(slices), expected))
    }

    fn read_hyperslab_raw<T: H5Type>(
        &mut self,
        dataset: &Dataset,
        selection: Hyperslab,
        expected: usize,
        purpose: &str,
    ) -> Result<Vec<T>, MatError> {
        let bytes = expected
            .checked_mul(size_of::<T>())
            .ok_or_else(|| limit(format!("{purpose} byte count overflow")))?;
        self.reserve(bytes)?;
        if expected == 0 {
            return Ok(Vec::new());
        }
        let file_space = dataset
            .space()
            .and_then(|space| space.select(selection))
            .map_err(|error| invalid(format!("invalid {purpose} selection: {error}")))?;
        let memory_space = Dataspace::try_new([expected])
            .map_err(|error| hdf5_error("cannot create hyperslab memory space", error))?;
        let datatype = Datatype::from_type::<T>()
            .map_err(|error| hdf5_error("cannot create hyperslab memory datatype", error))?;
        let mut values: Vec<T> = Vec::new();
        values
            .try_reserve_exact(expected)
            .map_err(|_| limit(format!("{purpose} allocation failed")))?;
        check_cancelled(self.cancellation)?;
        let status = hdf5::sync::sync(|| {
            // SAFETY: `values` owns capacity for `expected` T values; the selected
            // file and memory dataspaces contain exactly that many elements.
            unsafe {
                H5Dread(
                    dataset.id(),
                    datatype.id(),
                    memory_space.id(),
                    file_space.id(),
                    H5P_DEFAULT,
                    values.as_mut_ptr().cast::<c_void>(),
                )
            }
        });
        if status < 0 {
            return Err(hdf5_error(
                &format!("cannot read {purpose} from '{}'", dataset.name()),
                "H5Dread failed",
            ));
        }
        // SAFETY: H5Dread initialized exactly `expected` elements on success.
        unsafe { values.set_len(expected) };
        check_cancelled(self.cancellation)?;
        Ok(values)
    }

    fn read_integer_hyperslab<T: H5Type + openmat_array::IntegerElement>(
        &mut self,
        dataset: &Dataset,
        selection: Hyperslab,
        shape: Shape,
        expected: usize,
        wrap: impl FnOnce(DenseArray<T>) -> IntegerArrayData,
    ) -> Result<Value, MatError> {
        ensure_dataset_type::<T>(dataset, "integer hyperslab")?;
        let values =
            self.read_hyperslab_raw::<T>(dataset, selection, expected, "integer hyperslab")?;
        DenseArray::from_vec(shape, values)
            .map(wrap)
            .map(ArrayData::Integer)
            .map(Value::Array)
            .map_err(array_error)
    }

    #[allow(clippy::too_many_lines)]
    fn write_hyperslab_value(
        &mut self,
        dataset: &Dataset,
        selection: Hyperslab,
        expected: usize,
        class: &str,
        value: &Value,
    ) -> Result<(), MatError> {
        match class {
            "double" => {
                if dataset
                    .dtype()
                    .map_err(|error| hdf5_error("cannot inspect double type", error))?
                    .is::<CompoundF64>()
                {
                    let values = match value {
                        Value::Complex(value) => vec![CompoundF64 {
                            real: value.real,
                            imag: value.imaginary,
                        }],
                        Value::Double(value) => vec![CompoundF64 {
                            real: *value,
                            imag: 0.0,
                        }],
                        Value::Array(ArrayData::ComplexF64(array)) => self.stage_partial(
                            array.as_slice(),
                            "complex double hyperslab staging",
                            |value| CompoundF64 {
                                real: value.re,
                                imag: value.im,
                            },
                        )?,
                        Value::Array(ArrayData::F64(array)) => self.stage_partial(
                            array.as_slice(),
                            "real-to-complex hyperslab staging",
                            |value| CompoundF64 {
                                real: *value,
                                imag: 0.0,
                            },
                        )?,
                        _ => return Err(hyperslab_type_error(class, value)),
                    };
                    self.write_hyperslab_raw(dataset, selection, expected, &values)
                } else {
                    ensure_dataset_type::<f64>(dataset, "double")?;
                    let values: &[f64] = match value {
                        Value::Double(value) => std::slice::from_ref(value),
                        Value::Array(ArrayData::F64(array)) => array.as_slice(),
                        _ => return Err(hyperslab_type_error(class, value)),
                    };
                    self.write_hyperslab_raw(dataset, selection, expected, values)
                }
            }
            "single" => {
                if dataset
                    .dtype()
                    .map_err(|error| hdf5_error("cannot inspect single type", error))?
                    .is::<CompoundF32>()
                {
                    let values = match value {
                        Value::Array(ArrayData::ComplexF32(array)) => self.stage_partial(
                            array.as_slice(),
                            "complex single hyperslab staging",
                            |value| CompoundF32 {
                                real: value.re,
                                imag: value.im,
                            },
                        )?,
                        Value::Array(ArrayData::F32(array)) => self.stage_partial(
                            array.as_slice(),
                            "real-to-complex single hyperslab staging",
                            |value| CompoundF32 {
                                real: *value,
                                imag: 0.0,
                            },
                        )?,
                        _ => return Err(hyperslab_type_error(class, value)),
                    };
                    self.write_hyperslab_raw(dataset, selection, expected, &values)
                } else {
                    ensure_dataset_type::<f32>(dataset, "single")?;
                    let Value::Array(ArrayData::F32(array)) = value else {
                        return Err(hyperslab_type_error(class, value));
                    };
                    self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
                }
            }
            "logical" => {
                ensure_decode_attr(dataset, 1)?;
                ensure_dataset_type::<u8>(dataset, "logical")?;
                let values = match value {
                    Value::Logical(value) => vec![u8::from(*value)],
                    Value::Array(ArrayData::Logical(array)) => self.stage_partial(
                        array.as_slice(),
                        "logical hyperslab staging",
                        |value| u8::from(value.get()),
                    )?,
                    _ => return Err(hyperslab_type_error(class, value)),
                };
                self.write_hyperslab_raw(dataset, selection, expected, &values)
            }
            "char" => {
                ensure_decode_attr(dataset, 2)?;
                ensure_dataset_type::<u16>(dataset, "char")?;
                let Value::Array(ArrayData::Char(array)) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                let values =
                    self.stage_partial(array.as_slice(), "char hyperslab staging", |value| {
                        value.get()
                    })?;
                self.write_hyperslab_raw(dataset, selection, expected, &values)
            }
            "int8" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::I8(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            "uint8" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::U8(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            "int16" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::I16(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            "uint16" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::U16(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            "int32" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::I32(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            "uint32" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::U32(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            "int64" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::I64(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            "uint64" => {
                let Value::Array(ArrayData::Integer(IntegerArrayData::U64(array))) = value else {
                    return Err(hyperslab_type_error(class, value));
                };
                self.write_hyperslab_raw(dataset, selection, expected, array.as_slice())
            }
            other => Err(unsupported(format!(
                "MAT class '{other}' does not support dense hyperslab writes"
            ))),
        }
    }

    fn write_hyperslab_raw<T: H5Type>(
        &self,
        dataset: &Dataset,
        selection: Hyperslab,
        expected: usize,
        values: &[T],
    ) -> Result<(), MatError> {
        if values.len() != expected {
            return Err(invalid_value(format!(
                "hyperslab replacement has {} elements; expected {expected}",
                values.len()
            )));
        }
        if expected == 0 {
            return Ok(());
        }
        let file_space = dataset
            .space()
            .and_then(|space| space.select(selection))
            .map_err(|error| invalid_value(format!("invalid hyperslab selection: {error}")))?;
        let memory_space = Dataspace::try_new([expected])
            .map_err(|error| hdf5_error("cannot create hyperslab memory space", error))?;
        let datatype = Datatype::from_type::<T>()
            .map_err(|error| hdf5_error("cannot create hyperslab memory datatype", error))?;
        check_cancelled(self.cancellation)?;
        let status = hdf5::sync::sync(|| {
            // SAFETY: `values` contains exactly the number and type of elements
            // selected by the file and memory dataspaces.
            unsafe {
                H5Dwrite(
                    dataset.id(),
                    datatype.id(),
                    memory_space.id(),
                    file_space.id(),
                    H5P_DEFAULT,
                    values.as_ptr().cast::<c_void>(),
                )
            }
        });
        if status < 0 {
            return Err(hdf5_error(
                &format!("cannot write hyperslab to '{}'", dataset.name()),
                "H5Dwrite failed",
            ));
        }
        check_cancelled(self.cancellation)
    }

    fn stage_partial<T, U>(
        &mut self,
        input: &[T],
        purpose: &str,
        mut convert: impl FnMut(&T) -> U,
    ) -> Result<Vec<U>, MatError> {
        let bytes = input
            .len()
            .checked_mul(size_of::<U>())
            .ok_or_else(|| limit(format!("{purpose} byte count overflow")))?;
        self.reserve_allocation(bytes)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(input.len())
            .map_err(|_| limit(format!("{purpose} allocation failed")))?;
        for (index, value) in input.iter().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            values.push(convert(value));
        }
        Ok(values)
    }

    fn decode(mut self) -> Result<Vec<MatVariable>, MatError> {
        self.validate_hdf5_surface()?;
        check_cancelled(self.cancellation)?;
        let root = self
            .image
            .file
            .group("/")
            .map_err(|error| hdf5_error("cannot open MAT v7.3 root group", error))?;
        let mut names = root
            .member_names()
            .map_err(|error| hdf5_error("cannot enumerate MAT v7.3 variables", error))?;
        names.sort();
        let mut variables = Vec::new();
        variables
            .try_reserve(names.len().saturating_sub(1))
            .map_err(|_| limit("MAT v7.3 variable table allocation failed"))?;
        for (index, name) in names.into_iter().enumerate() {
            if name == "#refs#" || name == "#subsystem#" {
                continue;
            }
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            validate_hdf5_name(&name, "MAT variable")?;
            let value = match root
                .loc_type_by_name(&name)
                .map_err(|error| hdf5_error("cannot inspect MAT v7.3 root object", error))?
            {
                LocationType::Dataset => {
                    let dataset = root.dataset(&name).map_err(|error| {
                        hdf5_error(&format!("cannot open MAT variable '{name}'"), error)
                    })?;
                    self.decode_dataset(&dataset, 0)?
                }
                LocationType::Group => {
                    let group = root.group(&name).map_err(|error| {
                        hdf5_error(&format!("cannot open MAT variable '{name}'"), error)
                    })?;
                    self.decode_group(&group, 0)?
                }
                LocationType::NamedDatatype => {
                    return Err(unsupported(format!(
                        "MAT variable '{name}' is a named HDF5 datatype"
                    )));
                }
                LocationType::TypeMap => {
                    return Err(unsupported(format!("MAT variable '{name}' is an HDF5 map")));
                }
            };
            variables.push(MatVariable::new(name, value));
        }
        check_cancelled(self.cancellation)?;
        Ok(variables)
    }

    fn validate_hdf5_surface(&mut self) -> Result<(), MatError> {
        let root = self
            .image
            .file
            .group("/")
            .map_err(|error| hdf5_error("cannot open MAT v7.3 root for validation", error))?;
        let mut pending_groups = vec![(root, String::new(), 0usize)];
        let mut visited = 0usize;
        while let Some((group, prefix, group_depth)) = pending_groups.pop() {
            let remaining = self.limits.maximum_hdf5_objects.saturating_sub(visited);
            let remaining_allocation = self
                .limits
                .maximum_allocation_bytes
                .saturating_sub(self.allocated_bytes);
            let batch =
                collect_group_links(&group, remaining, remaining_allocation, self.cancellation)?;
            self.reserve_allocation(batch.allocation_bytes)?;
            for (name, link_type) in batch.entries {
                visited = visited
                    .checked_add(1)
                    .ok_or_else(|| limit("MAT v7.3 object count overflow"))?;
                if visited.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                    check_cancelled(self.cancellation)?;
                }
                let path_length = prefix
                    .len()
                    .checked_add(name.len())
                    .and_then(|length| length.checked_add(usize::from(!prefix.is_empty())))
                    .ok_or_else(|| limit("MAT v7.3 HDF5 path length overflow"))?;
                self.reserve_allocation(path_length)?;
                let mut path = String::new();
                path.try_reserve_exact(path_length)
                    .map_err(|_| limit("MAT v7.3 HDF5 path allocation failed"))?;
                if !prefix.is_empty() {
                    path.push_str(&prefix);
                    path.push('/');
                }
                path.push_str(&name);
                if link_type != LinkType::Hard {
                    return Err(unsupported(format!(
                        "MAT v7.3 object '/{path}' uses a forbidden {link_type:?} HDF5 link"
                    )));
                }
                validate_hdf5_name(&name, "HDF5 object")?;
                let info = group.loc_info_by_name(&name).map_err(|error| {
                    hdf5_error(&format!("cannot inspect HDF5 object '/{path}'"), error)
                })?;
                if info.num_links != 1 {
                    return Err(unsupported(format!(
                        "MAT v7.3 object '/{path}' has {} hard links; aliases and hard-link cycles are forbidden",
                        info.num_links
                    )));
                }
                match info.loc_type {
                    LocationType::Group => {
                        let child_depth = group_depth
                            .checked_add(1)
                            .ok_or_else(|| limit("MAT v7.3 HDF5 group depth overflow"))?;
                        let maximum_group_depth =
                            self.limits.maximum_recursion_depth.saturating_add(2);
                        if child_depth > maximum_group_depth {
                            return Err(limit(format!(
                                "MAT v7.3 HDF5 group nesting exceeds the configured {maximum_group_depth}-level physical limit"
                            )));
                        }
                        let child = group.group(&name).map_err(|error| {
                            hdf5_error(&format!("cannot open HDF5 group '/{path}'"), error)
                        })?;
                        pending_groups
                            .try_reserve(1)
                            .map_err(|_| limit("MAT v7.3 group traversal allocation failed"))?;
                        pending_groups.push((child, path, child_depth));
                    }
                    LocationType::Dataset => {
                        let dataset = group.dataset(&name).map_err(|error| {
                            hdf5_error(&format!("cannot open HDF5 dataset '/{path}'"), error)
                        })?;
                        validate_dataset_storage(&dataset, &path)?;
                    }
                    LocationType::NamedDatatype | LocationType::TypeMap => {}
                }
            }
        }
        Ok(())
    }

    fn decode_dataset(&mut self, dataset: &Dataset, depth: usize) -> Result<Value, MatError> {
        self.check_depth(depth)?;
        if read_optional_i32_attr(dataset, "MATLAB_object_decode")?.is_some() {
            let class = read_class(dataset).unwrap_or_else(|_| String::from("object"));
            return Err(unsupported(format!(
                "MAT object '{}' uses unsupported object-backed MATLAB_class '{class}'",
                dataset.name()
            )));
        }
        let class = read_class(dataset)?;
        if let Some(empty) = read_optional_u8_attr(dataset, "MATLAB_empty")? {
            if empty != 1 {
                return Err(invalid(format!(
                    "MAT object '{}' has invalid MATLAB_empty value {empty}",
                    dataset.name()
                )));
            }
            return self.decode_empty(dataset, &class);
        }
        let shape = dataset_shape(dataset)?;
        let expected = self.check_shape(&shape)?;
        match class.as_str() {
            "double" => self.decode_double(dataset, shape, expected),
            "single" => self.decode_single(dataset, shape, expected),
            "logical" => self.decode_logical(dataset, shape, expected),
            "char" => self.decode_char(dataset, shape, expected),
            "int8" => self.decode_integer::<i8>(dataset, shape, expected, "int8"),
            "uint8" => self.decode_integer::<u8>(dataset, shape, expected, "uint8"),
            "int16" => self.decode_integer::<i16>(dataset, shape, expected, "int16"),
            "uint16" => self.decode_integer::<u16>(dataset, shape, expected, "uint16"),
            "int32" => self.decode_integer::<i32>(dataset, shape, expected, "int32"),
            "uint32" => self.decode_integer::<u32>(dataset, shape, expected, "uint32"),
            "int64" => self.decode_integer::<i64>(dataset, shape, expected, "int64"),
            "uint64" => self.decode_integer::<u64>(dataset, shape, expected, "uint64"),
            "cell" => self.decode_cell(dataset, shape, expected, depth + 1),
            "struct" => Err(invalid(format!(
                "nonempty MAT struct '{}' must be an HDF5 group",
                dataset.name()
            ))),
            "canonical empty" => Ok(Value::empty_double()),
            other => Err(unsupported(format!(
                "MAT v7.3 object '{}' has unsupported MATLAB_class '{other}'",
                dataset.name()
            ))),
        }
    }

    fn decode_group(&mut self, group: &Group, depth: usize) -> Result<Value, MatError> {
        self.check_depth(depth)?;
        if read_optional_i32_attr(group, "MATLAB_object_decode")?.is_some() {
            let class = read_class(group).unwrap_or_else(|_| String::from("object"));
            return Err(unsupported(format!(
                "MAT group '{}' uses unsupported object-backed MATLAB_class '{class}'",
                group.name()
            )));
        }
        let class = read_class(group)?;
        if read_optional_u64_attr(group, "MATLAB_sparse")?.is_some() {
            return self.decode_sparse_group(group, &class);
        }
        match class.as_str() {
            "struct" => self.decode_struct_group(group, depth),
            other => Err(unsupported(format!(
                "MAT group '{}' has unsupported MATLAB_class '{other}'",
                group.name()
            ))),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn decode_sparse_group(&mut self, group: &Group, class: &str) -> Result<Value, MatError> {
        let rows = read_optional_u64_attr(group, "MATLAB_sparse")?.ok_or_else(|| {
            invalid(format!(
                "MAT sparse group '{}' is missing MATLAB_sparse",
                group.name()
            ))
        })?;
        let mut members = group
            .member_names()
            .map_err(|error| hdf5_error("cannot enumerate MAT sparse datasets", error))?;
        members.sort();
        if members
            .iter()
            .any(|name| name != "data" && name != "ir" && name != "jc")
        {
            return Err(unsupported(format!(
                "MAT sparse group '{}' contains unsupported datasets",
                group.name()
            )));
        }
        let jc = group.dataset("jc").map_err(|error| {
            invalid(format!(
                "MAT sparse group '{}' is missing column offsets 'jc': {error}",
                group.name()
            ))
        })?;
        ensure_vector_dataset(&jc, "sparse column offsets")?;
        ensure_dataset_type::<u64>(&jc, "sparse column offsets")?;
        let offset_count = jc.size();
        if offset_count == 0 {
            return Err(invalid(format!(
                "MAT sparse group '{}' has an empty 'jc' dataset",
                group.name()
            )));
        }
        if offset_count > self.limits.maximum_array_elements.saturating_add(1) {
            return Err(limit(format!(
                "MAT sparse group '{}' has too many column offsets",
                group.name()
            )));
        }
        let col_offsets = self.read_raw::<u64>(&jc, offset_count, "sparse column offsets")?;
        let columns = u64::try_from(offset_count - 1)
            .map_err(|_| limit("MAT sparse column count is not representable as u64"))?;
        let shape = Shape::new([rows, columns])
            .map_err(|error| invalid(format!("invalid MAT sparse shape: {error}")))?;
        self.check_shape(&shape)?;
        let nnz = usize::try_from(*col_offsets.last().expect("nonempty jc was checked"))
            .map_err(|_| limit("MAT sparse stored count is not host-representable"))?;
        if nnz > self.limits.maximum_array_elements {
            return Err(limit(format!(
                "MAT sparse group '{}' has {nnz} stored entries, exceeding the configured limit",
                group.name()
            )));
        }

        let has_ir = members
            .binary_search_by(|name| name.as_str().cmp("ir"))
            .is_ok();
        let has_data = members
            .binary_search_by(|name| name.as_str().cmp("data"))
            .is_ok();
        if has_ir != has_data || (nnz != 0 && !has_ir) {
            return Err(invalid(format!(
                "MAT sparse group '{}' has inconsistent 'ir' and 'data' datasets",
                group.name()
            )));
        }
        let row_indices = if has_ir {
            let ir = group
                .dataset("ir")
                .map_err(|error| hdf5_error("cannot open MAT sparse row indices", error))?;
            ensure_vector_dataset(&ir, "sparse row indices")?;
            ensure_dataset_type::<u64>(&ir, "sparse row indices")?;
            self.read_raw::<u64>(&ir, nnz, "sparse row indices")?
        } else {
            Vec::new()
        };

        match class {
            "double" => {
                if !has_data {
                    return CscMatrix::<f64>::try_from_canonical_parts(
                        rows,
                        columns,
                        col_offsets,
                        row_indices,
                        Vec::new(),
                        0,
                        self.cancellation,
                    )
                    .map(SparseArrayData::F64)
                    .map(Value::Sparse)
                    .map_err(sparse_error);
                }
                let data = group
                    .dataset("data")
                    .map_err(|error| hdf5_error("cannot open MAT sparse values", error))?;
                ensure_vector_dataset(&data, "sparse values")?;
                if data
                    .dtype()
                    .map_err(|error| hdf5_error("cannot inspect sparse value type", error))?
                    .is::<CompoundF64>()
                {
                    let raw = self.read_raw::<CompoundF64>(&data, nnz, "complex sparse values")?;
                    let values =
                        self.convert_values(raw, "complex sparse conversion", |value| {
                            Complex64::new(value.real, value.imag)
                        })?;
                    CscMatrix::try_from_canonical_parts(
                        rows,
                        columns,
                        col_offsets,
                        row_indices,
                        values,
                        nnz,
                        self.cancellation,
                    )
                    .map(SparseArrayData::ComplexF64)
                    .map(Value::Sparse)
                    .map_err(sparse_error)
                } else {
                    ensure_dataset_type::<f64>(&data, "sparse double values")?;
                    let values = self.read_raw::<f64>(&data, nnz, "sparse double values")?;
                    CscMatrix::try_from_canonical_parts(
                        rows,
                        columns,
                        col_offsets,
                        row_indices,
                        values,
                        nnz,
                        self.cancellation,
                    )
                    .map(SparseArrayData::F64)
                    .map(Value::Sparse)
                    .map_err(sparse_error)
                }
            }
            "logical" => {
                ensure_decode_attr(group, 1)?;
                let values = if has_data {
                    let data = group.dataset("data").map_err(|error| {
                        hdf5_error("cannot open MAT sparse logical values", error)
                    })?;
                    ensure_vector_dataset(&data, "sparse logical values")?;
                    ensure_dataset_type::<u8>(&data, "sparse logical values")?;
                    let raw = self.read_raw::<u8>(&data, nnz, "sparse logical values")?;
                    self.convert_values(raw, "sparse logical conversion", |value| {
                        Logical::from(value != 0)
                    })?
                } else {
                    Vec::new()
                };
                CscMatrix::try_from_canonical_parts(
                    rows,
                    columns,
                    col_offsets,
                    row_indices,
                    values,
                    nnz,
                    self.cancellation,
                )
                .map(SparseArrayData::Logical)
                .map(Value::Sparse)
                .map_err(sparse_error)
            }
            other => Err(unsupported(format!(
                "MAT sparse group '{}' has unsupported MATLAB_class '{other}'",
                group.name()
            ))),
        }
    }

    fn decode_empty(&mut self, dataset: &Dataset, class: &str) -> Result<Value, MatError> {
        ensure_dataset_type::<u64>(dataset, "empty shape")?;
        let dimensions = self.read_raw::<u64>(dataset, dataset.size(), "empty shape")?;
        if dimensions.len() < 2 {
            return Err(invalid(format!(
                "empty MAT object '{}' stores fewer than two dimensions",
                dataset.name()
            )));
        }
        let shape = Shape::new(dimensions)
            .map_err(|error| invalid(format!("invalid empty MAT shape: {error}")))?;
        self.check_shape(&shape)?;
        match class {
            "double" => empty_array(shape, ArrayData::F64),
            "single" => DenseArray::from_vec(shape, Vec::<f32>::new())
                .map(ArrayData::F32)
                .map(Value::Array)
                .map_err(array_error),
            "logical" => empty_array(shape, ArrayData::Logical),
            "char" => empty_array(shape, ArrayData::Char),
            "int8" => empty_integer::<i8>(shape),
            "uint8" => empty_integer::<u8>(shape),
            "int16" => empty_integer::<i16>(shape),
            "uint16" => empty_integer::<u16>(shape),
            "int32" => empty_integer::<i32>(shape),
            "uint32" => empty_integer::<u32>(shape),
            "int64" => empty_integer::<i64>(shape),
            "uint64" => empty_integer::<u64>(shape),
            "cell" => CellArray::from_values(shape, Vec::new())
                .map(Value::Cell)
                .map_err(|error| invalid(format!("invalid empty MAT cell: {error}"))),
            "struct" => {
                let fields = read_fields(dataset)?;
                StructArray::empty(shape, fields)
                    .map(Value::Struct)
                    .map_err(|error| invalid(format!("invalid empty MAT struct: {error}")))
            }
            "canonical empty" => Ok(Value::empty_double()),
            other => Err(unsupported(format!(
                "empty MAT object '{}' has unsupported MATLAB_class '{other}'",
                dataset.name()
            ))),
        }
    }

    fn decode_double(
        &mut self,
        dataset: &Dataset,
        shape: Shape,
        expected: usize,
    ) -> Result<Value, MatError> {
        if dataset
            .dtype()
            .map_err(|error| hdf5_error("cannot inspect double type", error))?
            .is::<CompoundF64>()
        {
            let values = self.read_raw::<CompoundF64>(dataset, expected, "complex double")?;
            if expected == 1 {
                return Ok(Value::Complex(openmat_value::Complex64::new(
                    values[0].real,
                    values[0].imag,
                )));
            }
            let values = self.convert_values(values, "complex double conversion", |value| {
                Complex64::new(value.real, value.imag)
            })?;
            DenseArray::from_vec(shape, values)
                .map(ArrayData::ComplexF64)
                .map(Value::Array)
                .map_err(array_error)
        } else {
            ensure_dataset_type::<f64>(dataset, "double")?;
            let values = self.read_raw::<f64>(dataset, expected, "double")?;
            if expected == 1 {
                return Ok(Value::Double(values[0]));
            }
            DenseArray::from_vec(shape, values)
                .map(ArrayData::F64)
                .map(Value::Array)
                .map_err(array_error)
        }
    }

    fn decode_single(
        &mut self,
        dataset: &Dataset,
        shape: Shape,
        expected: usize,
    ) -> Result<Value, MatError> {
        if dataset
            .dtype()
            .map_err(|error| hdf5_error("cannot inspect single type", error))?
            .is::<CompoundF32>()
        {
            let values = self.read_raw::<CompoundF32>(dataset, expected, "complex single")?;
            let values = self.convert_values(values, "complex single conversion", |value| {
                Complex32::new(value.real, value.imag)
            })?;
            DenseArray::from_vec(shape, values)
                .map(ArrayData::ComplexF32)
                .map(Value::Array)
                .map_err(array_error)
        } else {
            ensure_dataset_type::<f32>(dataset, "single")?;
            let values = self.read_raw::<f32>(dataset, expected, "single")?;
            DenseArray::from_vec(shape, values)
                .map(ArrayData::F32)
                .map(Value::Array)
                .map_err(array_error)
        }
    }

    fn decode_logical(
        &mut self,
        dataset: &Dataset,
        shape: Shape,
        expected: usize,
    ) -> Result<Value, MatError> {
        ensure_decode_attr(dataset, 1)?;
        ensure_dataset_type::<u8>(dataset, "logical")?;
        let raw = self.read_raw::<u8>(dataset, expected, "logical")?;
        if expected == 1 {
            return Ok(Value::Logical(raw[0] != 0));
        }
        let values =
            self.convert_values(raw, "logical conversion", |value| Logical::from(value != 0))?;
        DenseArray::from_vec(shape, values)
            .map(ArrayData::Logical)
            .map(Value::Array)
            .map_err(array_error)
    }

    fn decode_char(
        &mut self,
        dataset: &Dataset,
        shape: Shape,
        expected: usize,
    ) -> Result<Value, MatError> {
        ensure_decode_attr(dataset, 2)?;
        ensure_dataset_type::<u16>(dataset, "char")?;
        let raw = self.read_raw::<u16>(dataset, expected, "char")?;
        let values = self.convert_values(raw, "char conversion", CharCodeUnit::new)?;
        DenseArray::from_vec(shape, values)
            .map(ArrayData::Char)
            .map(Value::Array)
            .map_err(array_error)
    }

    fn decode_integer<T: Mat73Integer>(
        &mut self,
        dataset: &Dataset,
        shape: Shape,
        expected: usize,
        class: &str,
    ) -> Result<Value, MatError> {
        T::decode(self, dataset, shape, expected).map_err(|error| {
            if error.kind() == MatErrorKind::InvalidFormat {
                invalid(format!(
                    "invalid {class} MAT dataset '{}': {error}",
                    dataset.name()
                ))
            } else {
                error
            }
        })
    }

    fn decode_cell(
        &mut self,
        dataset: &Dataset,
        shape: Shape,
        expected: usize,
        depth: usize,
    ) -> Result<Value, MatError> {
        ensure_dataset_type::<ObjectReference1>(dataset, "cell references")?;
        self.reference_count = self
            .reference_count
            .checked_add(expected)
            .ok_or_else(|| limit("MAT v7.3 reference count overflow"))?;
        if self.reference_count > self.limits.maximum_hdf5_references {
            return Err(limit(format!(
                "MAT v7.3 reference count exceeds the configured {}-reference limit",
                self.limits.maximum_hdf5_references
            )));
        }
        let references = self.read_raw::<ObjectReference1>(dataset, expected, "cell references")?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(expected)
            .map_err(|_| limit("MAT v7.3 cell allocation failed"))?;
        for (index, reference) in references.iter().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            values.push(self.decode_reference(*reference, depth)?);
        }
        CellArray::from_values(shape, values)
            .map(Value::Cell)
            .map_err(|error| invalid(format!("invalid MAT cell array: {error}")))
    }

    #[allow(clippy::too_many_lines)]
    fn decode_struct_group(&mut self, group: &Group, depth: usize) -> Result<Value, MatError> {
        self.check_depth(depth)?;
        let class = read_class(group)?;
        if class != "struct" {
            return Err(unsupported(format!(
                "MAT group '{}' has unsupported MATLAB_class '{class}'",
                group.name()
            )));
        }
        let fields = read_fields(group)?;
        let mut shape = None;
        let mut columns = Vec::new();
        columns
            .try_reserve_exact(fields.len())
            .map_err(|_| limit("MAT v7.3 struct column allocation failed"))?;
        for (field_index, field) in fields.iter().enumerate() {
            if field_index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            let name = field.as_str();
            validate_hdf5_name(name, "MAT struct field")?;
            let field_type = group.loc_type_by_name(name).map_err(|error| {
                invalid(format!(
                    "MAT struct '{}' is missing field '{name}': {error}",
                    group.name()
                ))
            })?;
            let (field_shape, column) = match field_type {
                LocationType::Group => {
                    let nested = group.group(name).map_err(|error| {
                        hdf5_error(
                            &format!("cannot open inline MAT struct field '{name}'"),
                            error,
                        )
                    })?;
                    let value = self.decode_group(&nested, depth + 1)?;
                    (
                        Shape::new([1, 1]).expect("scalar struct shape"),
                        vec![value],
                    )
                }
                LocationType::Dataset => {
                    let dataset = group.dataset(name).map_err(|error| {
                        hdf5_error(&format!("cannot open MAT struct field '{name}'"), error)
                    })?;
                    if dataset
                        .dtype()
                        .map_err(|error| hdf5_error("cannot inspect MAT struct field", error))?
                        .is::<ObjectReference1>()
                    {
                        let field_shape = dataset_shape(&dataset)?;
                        let expected = self.check_shape(&field_shape)?;
                        self.reference_count = self
                            .reference_count
                            .checked_add(expected)
                            .ok_or_else(|| limit("MAT v7.3 reference count overflow"))?;
                        if self.reference_count > self.limits.maximum_hdf5_references {
                            return Err(limit(format!(
                                "MAT v7.3 reference count exceeds the configured {}-reference limit",
                                self.limits.maximum_hdf5_references
                            )));
                        }
                        let references = self.read_raw::<ObjectReference1>(
                            &dataset,
                            expected,
                            "struct field references",
                        )?;
                        let mut column = Vec::new();
                        column
                            .try_reserve_exact(expected)
                            .map_err(|_| limit("MAT v7.3 struct field allocation failed"))?;
                        for reference in &references {
                            column.push(self.decode_reference(*reference, depth + 1)?);
                        }
                        (field_shape, column)
                    } else {
                        let value = self.decode_dataset(&dataset, depth + 1)?;
                        (
                            Shape::new([1, 1]).expect("scalar struct shape"),
                            vec![value],
                        )
                    }
                }
                LocationType::NamedDatatype | LocationType::TypeMap => {
                    return Err(unsupported(format!(
                        "MAT struct '{}' field '{name}' is not a value object",
                        group.name()
                    )));
                }
            };
            if shape
                .as_ref()
                .is_some_and(|current| current != &field_shape)
            {
                return Err(invalid(format!(
                    "MAT struct '{}' field datasets have inconsistent shapes",
                    group.name()
                )));
            }
            self.check_shape(&field_shape)?;
            if shape.is_none() {
                shape = Some(field_shape);
            }
            columns.push(column);
        }
        let shape = shape.ok_or_else(|| {
            invalid(format!(
                "nonempty MAT struct '{}' has no field shape",
                group.name()
            ))
        })?;
        StructArray::from_columns(shape, fields, columns)
            .map(Value::Struct)
            .map_err(|error| invalid(format!("invalid MAT struct array: {error}")))
    }

    fn decode_reference(
        &mut self,
        reference: ObjectReference1,
        depth: usize,
    ) -> Result<Value, MatError> {
        self.check_depth(depth)?;
        let object = self
            .image
            .file
            .dereference(&reference)
            .map_err(|error| invalid(format!("invalid MAT v7.3 object reference: {error}")))?;
        let path = match &object {
            ReferencedObject::Dataset(dataset) => dataset.name(),
            ReferencedObject::Group(group) => group.name(),
            ReferencedObject::Datatype(_) => {
                return Err(unsupported(
                    "MAT v7.3 reference targets a named HDF5 datatype",
                ));
            }
        };
        if !path.starts_with("/#refs#/") {
            return Err(invalid(format!(
                "MAT v7.3 reference targets '{path}' outside '/#refs#/'"
            )));
        }
        if let Some(value) = self.cached_references.get(&path) {
            return Ok(value.clone());
        }
        if !self.active_references.insert(path.clone()) {
            return Err(invalid(format!(
                "MAT v7.3 reference cycle detected at '{path}'"
            )));
        }
        let value = match object {
            ReferencedObject::Dataset(dataset) => self.decode_dataset(&dataset, depth),
            ReferencedObject::Group(group) => self.decode_group(&group, depth),
            ReferencedObject::Datatype(_) => Err(unsupported(format!(
                "MAT v7.3 reference '{path}' targets a named datatype"
            ))),
        };
        self.active_references.remove(&path);
        let value = value?;
        self.cached_references.insert(path, value.clone());
        Ok(value)
    }

    fn read_raw<T: H5Type>(
        &mut self,
        dataset: &Dataset,
        expected: usize,
        purpose: &str,
    ) -> Result<Vec<T>, MatError> {
        let bytes = expected
            .checked_mul(size_of::<T>())
            .ok_or_else(|| limit(format!("{purpose} byte count overflow")))?;
        self.reserve(bytes)?;
        check_cancelled(self.cancellation)?;
        let values = dataset.read_raw::<T>().map_err(|error| {
            hdf5_error(
                &format!("cannot read {purpose} from '{}'", dataset.name()),
                error,
            )
        })?;
        check_cancelled(self.cancellation)?;
        if values.len() != expected {
            return Err(invalid(format!(
                "MAT dataset '{}' contains {} {purpose} elements; expected {expected}",
                dataset.name(),
                values.len()
            )));
        }
        Ok(values)
    }

    fn convert_values<T, U>(
        &mut self,
        values: Vec<T>,
        purpose: &str,
        mut convert: impl FnMut(T) -> U,
    ) -> Result<Vec<U>, MatError> {
        let bytes = values
            .len()
            .checked_mul(size_of::<U>())
            .ok_or_else(|| limit(format!("{purpose} byte count overflow")))?;
        self.reserve_allocation(bytes)?;
        let mut converted = Vec::new();
        converted
            .try_reserve_exact(values.len())
            .map_err(|_| limit(format!("{purpose} allocation failed")))?;
        for (index, value) in values.into_iter().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            converted.push(convert(value));
        }
        Ok(converted)
    }

    fn reserve(&mut self, bytes: usize) -> Result<(), MatError> {
        self.reserve_allocation(bytes)?;
        self.decompressed_bytes = self
            .decompressed_bytes
            .checked_add(bytes)
            .ok_or_else(|| limit("MAT v7.3 decompressed byte count overflow"))?;
        if self.decompressed_bytes > self.limits.maximum_hdf5_decompressed_bytes {
            return Err(limit(format!(
                "MAT v7.3 decoded datasets exceed the configured {}-byte decompression limit",
                self.limits.maximum_hdf5_decompressed_bytes
            )));
        }
        Ok(())
    }

    fn reserve_allocation(&mut self, bytes: usize) -> Result<(), MatError> {
        self.allocated_bytes = self
            .allocated_bytes
            .checked_add(bytes)
            .ok_or_else(|| limit("MAT v7.3 allocation byte count overflow"))?;
        if self.allocated_bytes > self.limits.maximum_allocation_bytes {
            return Err(limit(format!(
                "MAT v7.3 decoded allocation exceeds the configured {}-byte limit",
                self.limits.maximum_allocation_bytes
            )));
        }
        Ok(())
    }

    fn check_shape(&self, shape: &Shape) -> Result<usize, MatError> {
        let elements = usize::try_from(shape.numel())
            .map_err(|_| limit("MAT v7.3 array element count is not host-representable"))?;
        if elements > self.limits.maximum_array_elements {
            return Err(limit(format!(
                "MAT v7.3 array has {elements} elements, exceeding the configured {}-element limit",
                self.limits.maximum_array_elements
            )));
        }
        Ok(elements)
    }

    fn check_depth(&self, depth: usize) -> Result<(), MatError> {
        if depth > self.limits.maximum_recursion_depth {
            Err(limit(format!(
                "MAT v7.3 nesting exceeds the configured {}-level limit",
                self.limits.maximum_recursion_depth
            )))
        } else {
            Ok(())
        }
    }
}

fn validate_dataset_storage(dataset: &Dataset, path: &str) -> Result<(), MatError> {
    let creation = dataset
        .dcpl()
        .map_err(|error| hdf5_error("cannot inspect HDF5 dataset storage", error))?;
    if !creation.external().is_empty() || creation.layout() == Layout::Virtual {
        return Err(unsupported(format!(
            "MAT v7.3 dataset '/{path}' uses forbidden external HDF5 storage"
        )));
    }
    if dataset
        .filters()
        .iter()
        .any(|filter| !matches!(filter, Filter::Deflate(0..=9)))
    {
        return Err(unsupported(format!(
            "MAT v7.3 dataset '/{path}' uses a filter other than built-in zlib deflate"
        )));
    }
    Ok(())
}

fn empty_array<T: Default + Clone>(
    shape: Shape,
    wrap: impl FnOnce(DenseArray<T>) -> ArrayData,
) -> Result<Value, MatError> {
    DenseArray::from_vec(shape, Vec::new())
        .map(wrap)
        .map(Value::Array)
        .map_err(array_error)
}

fn empty_integer<T: openmat_array::IntegerElement>(shape: Shape) -> Result<Value, MatError> {
    DenseArray::<T>::from_vec(shape, Vec::new())
        .map(IntegerArrayData::from_typed)
        .map(ArrayData::Integer)
        .map(Value::Array)
        .map_err(array_error)
}

fn dense_f64_value(shape: Shape, values: Vec<f64>) -> Result<Value, MatError> {
    if values.len() == 1 {
        Ok(Value::Double(values[0]))
    } else {
        DenseArray::from_vec(shape, values)
            .map(ArrayData::F64)
            .map(Value::Array)
            .map_err(array_error)
    }
}

fn dense_complex_value(shape: Shape, values: Vec<Complex64>) -> Result<Value, MatError> {
    if values.len() == 1 {
        Ok(Value::Complex(openmat_value::Complex64::new(
            values[0].re,
            values[0].im,
        )))
    } else {
        DenseArray::from_vec(shape, values)
            .map(ArrayData::ComplexF64)
            .map(Value::Array)
            .map_err(array_error)
    }
}

fn dense_logical_value(shape: Shape, values: Vec<Logical>) -> Result<Value, MatError> {
    if values.len() == 1 {
        Ok(Value::Logical(values[0].get()))
    } else {
        DenseArray::from_vec(shape, values)
            .map(ArrayData::Logical)
            .map(Value::Array)
            .map_err(array_error)
    }
}

fn hyperslab_type_error(class: &str, value: &Value) -> MatError {
    invalid_value(format!(
        "cannot write runtime class '{}' into a MAT {class} hyperslab without changing its storage type",
        value.class_name()
    ))
}

#[allow(clippy::needless_pass_by_value)]
fn array_error(error: openmat_array::ArrayError) -> MatError {
    invalid(format!("invalid MAT array storage: {error}"))
}

#[allow(clippy::needless_pass_by_value)]
fn sparse_error(error: openmat_value::SparseError) -> MatError {
    invalid(format!("invalid MAT sparse CSC storage: {error}"))
}

fn validate_hdf5_name(name: &str, purpose: &str) -> Result<(), MatError> {
    validate_name(name)?;
    if name.len() > MAX_NAME_BYTES {
        return Err(limit(format!(
            "{purpose} name is {} bytes, exceeding the {MAX_NAME_BYTES}-byte limit",
            name.len()
        )));
    }
    if name.contains('/') {
        return Err(invalid_value(format!(
            "{purpose} name '{name}' contains the HDF5 path separator"
        )));
    }
    Ok(())
}

fn dataset_shape(dataset: &Dataset) -> Result<Shape, MatError> {
    let mut dimensions = dataset
        .shape()
        .into_iter()
        .map(|dimension| {
            u64::try_from(dimension)
                .map_err(|_| limit("MAT v7.3 dimension is not representable as u64"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    dimensions.reverse();
    if dimensions.len() < 2 {
        return Err(invalid(format!(
            "MAT dataset '{}' has fewer than two dimensions",
            dataset.name()
        )));
    }
    Shape::new(dimensions).map_err(|error| invalid(format!("invalid MAT array shape: {error}")))
}

fn ensure_dataset_type<T: H5Type>(dataset: &Dataset, purpose: &str) -> Result<(), MatError> {
    let datatype = dataset
        .dtype()
        .map_err(|error| hdf5_error(&format!("cannot inspect {purpose} datatype"), error))?;
    if datatype.is::<T>() {
        Ok(())
    } else {
        Err(invalid(format!(
            "MAT dataset '{}' does not use the required {purpose} HDF5 datatype",
            dataset.name()
        )))
    }
}

fn ensure_vector_dataset(dataset: &Dataset, purpose: &str) -> Result<(), MatError> {
    if dataset.shape().len() == 1 {
        Ok(())
    } else {
        Err(invalid(format!(
            "MAT dataset '{}' does not use one-dimensional {purpose} storage",
            dataset.name()
        )))
    }
}

fn read_class(location: &Location) -> Result<String, MatError> {
    let attribute = location.attr("MATLAB_class").map_err(|error| {
        invalid(format!(
            "MAT object '{}' is missing scalar ASCII attribute MATLAB_class: {error}",
            location.name()
        ))
    })?;
    let datatype = attribute.dtype().map_err(|error| {
        invalid(format!(
            "MAT object '{}' has an invalid MATLAB_class datatype: {error}",
            location.name()
        ))
    })?;
    macro_rules! fixed {
        ($length:literal) => {
            attribute
                .read_scalar::<FixedAscii<$length>>()
                .map(|value| value.as_str().to_owned())
        };
    }
    let result = match datatype.size() {
        4 => fixed!(4),
        5 => fixed!(5),
        6 => fixed!(6),
        7 => fixed!(7),
        15 => fixed!(15),
        _ if datatype.is::<VarLenAscii>() => attribute
            .read_scalar::<VarLenAscii>()
            .map(|value| value.as_str().to_owned()),
        size => {
            return Err(invalid(format!(
                "MAT object '{}' has unsupported {size}-byte MATLAB_class storage",
                location.name()
            )));
        }
    };
    result.map_err(|error| {
        invalid(format!(
            "MAT object '{}' has an invalid MATLAB_class attribute: {error}",
            location.name()
        ))
    })
}

fn read_fields(location: &Location) -> Result<Vec<FieldName>, MatError> {
    let Ok(attribute) = location.attr("MATLAB_fields") else {
        return Ok(Vec::new());
    };
    let raw = attribute
        .read_raw::<VarLenArray<FixedAscii<1>>>()
        .map_err(|error| {
            invalid(format!(
                "MAT struct '{}' has an invalid MATLAB_fields attribute: {error}",
                location.name()
            ))
        })?;
    let mut fields = Vec::new();
    fields
        .try_reserve_exact(raw.len())
        .map_err(|_| limit("MAT struct field table allocation failed"))?;
    for characters in raw {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(characters.len())
            .map_err(|_| limit("MAT struct field name allocation failed"))?;
        for character in characters.iter() {
            bytes.extend_from_slice(character.as_bytes());
        }
        let name = std::str::from_utf8(&bytes)
            .map_err(|_| invalid("MAT struct field name is not valid UTF-8"))?;
        validate_hdf5_name(name, "MAT struct field")?;
        fields.push(
            FieldName::new(name)
                .map_err(|error| invalid(format!("invalid MAT struct field name: {error}")))?,
        );
    }
    Ok(fields)
}

fn read_optional_u8_attr(location: &Location, name: &str) -> Result<Option<u8>, MatError> {
    let Ok(attribute) = location.attr(name) else {
        return Ok(None);
    };
    attribute.read_scalar::<u8>().map(Some).map_err(|error| {
        invalid(format!(
            "MAT object '{}' has an invalid {name} attribute: {error}",
            location.name()
        ))
    })
}

fn read_optional_u64_attr(location: &Location, name: &str) -> Result<Option<u64>, MatError> {
    let Ok(attribute) = location.attr(name) else {
        return Ok(None);
    };
    attribute.read_scalar::<u64>().map(Some).map_err(|error| {
        invalid(format!(
            "MAT object '{}' has an invalid {name} attribute: {error}",
            location.name()
        ))
    })
}

fn read_optional_i32_attr(location: &Location, name: &str) -> Result<Option<i32>, MatError> {
    let Ok(attribute) = location.attr(name) else {
        return Ok(None);
    };
    attribute.read_scalar::<i32>().map(Some).map_err(|error| {
        invalid(format!(
            "MAT object '{}' has an invalid {name} attribute: {error}",
            location.name()
        ))
    })
}

fn ensure_decode_attr(location: &Location, expected: i32) -> Result<(), MatError> {
    let attribute = location.attr("MATLAB_int_decode").map_err(|error| {
        invalid(format!(
            "MAT object '{}' is missing MATLAB_int_decode: {error}",
            location.name()
        ))
    })?;
    let actual = attribute.read_scalar::<i32>().map_err(|error| {
        invalid(format!(
            "MAT object '{}' has invalid MATLAB_int_decode: {error}",
            location.name()
        ))
    })?;
    if actual != expected {
        return Err(invalid(format!(
            "MAT object '{}' has MATLAB_int_decode={actual}; expected {expected}",
            location.name()
        )));
    }
    Ok(())
}

trait Mat73Integer: openmat_array::IntegerElement + H5Type {
    fn decode(
        decoder: &mut Decoder<'_>,
        dataset: &Dataset,
        shape: Shape,
        expected: usize,
    ) -> Result<Value, MatError>;

    fn encode(
        encoder: &mut Encoder<'_>,
        parent: &Group,
        name: &str,
        value: &IntegerArrayData,
    ) -> Result<Dataset, MatError>;
}

macro_rules! impl_mat73_integer {
    ($ty:ty, $real:ident, $complex:ident, $compound:ident) => {
        impl Mat73Integer for $ty {
            fn decode(
                decoder: &mut Decoder<'_>,
                dataset: &Dataset,
                shape: Shape,
                expected: usize,
            ) -> Result<Value, MatError> {
                let integer = if dataset
                    .dtype()
                    .map_err(|error| hdf5_error("cannot inspect integer type", error))?
                    .is::<$compound>()
                {
                    let values =
                        decoder.read_raw::<$compound>(dataset, expected, "complex integer")?;
                    let values =
                        decoder.convert_values(values, "complex integer conversion", |value| {
                            ComplexInteger::new(value.real, value.imag)
                        })?;
                    IntegerArrayData::$complex(
                        DenseArray::from_vec(shape, values).map_err(array_error)?,
                    )
                } else {
                    ensure_dataset_type::<$ty>(dataset, "integer")?;
                    let values = decoder.read_raw::<$ty>(dataset, expected, "integer")?;
                    IntegerArrayData::$real(
                        DenseArray::from_vec(shape, values).map_err(array_error)?,
                    )
                };
                Ok(Value::Array(ArrayData::Integer(integer)))
            }

            fn encode(
                encoder: &mut Encoder<'_>,
                parent: &Group,
                name: &str,
                value: &IntegerArrayData,
            ) -> Result<Dataset, MatError> {
                match value {
                    IntegerArrayData::$real(array) => {
                        encoder.write_dataset(parent, name, array.shape(), array.as_slice(), true)
                    }
                    IntegerArrayData::$complex(array) => {
                        let values = encoder.stage_values(
                            array.as_slice(),
                            "complex integer staging",
                            |value| $compound {
                                real: value.re(),
                                imag: value.im(),
                            },
                        )?;
                        encoder.write_dataset(parent, name, array.shape(), &values, true)
                    }
                    _ => Err(invalid_value("integer encoder class mismatch")),
                }
            }
        }
    };
}

impl_mat73_integer!(i8, I8, ComplexI8, CompoundI8);
impl_mat73_integer!(u8, U8, ComplexU8, CompoundU8);
impl_mat73_integer!(i16, I16, ComplexI16, CompoundI16);
impl_mat73_integer!(u16, U16, ComplexU16, CompoundU16);
impl_mat73_integer!(i32, I32, ComplexI32, CompoundI32);
impl_mat73_integer!(u32, U32, ComplexU32, CompoundU32);
impl_mat73_integer!(i64, I64, ComplexI64, CompoundI64);
impl_mat73_integer!(u64, U64, ComplexU64, CompoundU64);

struct Encoder<'a> {
    file: File,
    root: Group,
    refs: Group,
    limits: MatLimits,
    cancellation: Option<&'a AtomicBool>,
    estimated_bytes: usize,
    object_count: usize,
    reference_count: usize,
    next_reference: u64,
}

impl<'a> Encoder<'a> {
    fn new(limits: MatLimits, cancellation: Option<&'a AtomicBool>) -> Result<Self, MatError> {
        check_cancelled(cancellation)?;
        if limits.maximum_file_bytes < MATLAB_USER_BLOCK_BYTES {
            return Err(limit(format!(
                "MAT v7.3 output needs at least a {MATLAB_USER_BLOCK_BYTES}-byte user block"
            )));
        }
        if limits.maximum_allocation_bytes < MATLAB_USER_BLOCK_BYTES {
            return Err(limit(format!(
                "MAT v7.3 encoder allocation limit is smaller than its {MATLAB_USER_BLOCK_BYTES}-byte user block"
            )));
        }
        if limits.maximum_hdf5_objects < 1 {
            return Err(limit(
                "MAT v7.3 encoder object limit cannot hold the required '/#refs#/' group",
            ));
        }
        disable_dynamic_plugins()?;
        let access = file_access()?;
        let sequence = IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!("openmat-mat73-write-{sequence}");
        let mut builder = File::with_options();
        builder
            .set_fapl(&access)
            .map_err(|error| hdf5_error("cannot apply MAT v7.3 file access properties", error))?;
        builder.with_fcpl(|properties| properties.userblock(MATLAB_USER_BLOCK_BYTES as u64));
        let file = builder
            .create(name)
            .map_err(|error| hdf5_error("cannot create in-memory MAT v7.3 file", error))?;
        let root = file
            .group("/")
            .map_err(|error| hdf5_error("cannot open new MAT v7.3 root", error))?;
        let refs = root
            .create_group("#refs#")
            .map_err(|error| hdf5_error("cannot create MAT v7.3 reference group", error))?;
        Ok(Self {
            file,
            root,
            refs,
            limits,
            cancellation,
            estimated_bytes: MATLAB_USER_BLOCK_BYTES,
            object_count: 1,
            reference_count: 0,
            next_reference: 0,
        })
    }

    fn encode(mut self, variables: &[MatVariable]) -> Result<Vec<u8>, MatError> {
        for (index, variable) in variables.iter().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            validate_hdf5_name(&variable.name, "MAT variable")
                .map_err(|error| variable_error(&variable.name, error))?;
            let root = self.root.clone();
            self.write_value(&root, &variable.name, &variable.value, 0)
                .map_err(|error| variable_error(&variable.name, error))?;
        }
        check_cancelled(self.cancellation)?;
        let image = extract_image(
            &self.file,
            self.limits.maximum_file_bytes,
            self.limits.maximum_allocation_bytes,
        )?;
        check_cancelled(self.cancellation)?;
        Ok(image)
    }

    fn write_value(
        &mut self,
        parent: &Group,
        name: &str,
        value: &Value,
        depth: usize,
    ) -> Result<(), MatError> {
        self.check_depth(depth)?;
        check_cancelled(self.cancellation)?;
        if should_use_empty_representation(value) {
            let dataset = self.write_empty(parent, name, value)?;
            if let Value::Struct(structure) = value {
                self.write_fields(&dataset, structure.field_names())?;
            }
            return Ok(());
        }
        match value {
            Value::Logical(value) => {
                let dataset = self.write_dataset(
                    parent,
                    name,
                    &Shape::new([1, 1]).expect("scalar shape"),
                    &[u8::from(*value)],
                    false,
                )?;
                write_class(&dataset, "logical")?;
                write_i32_attr(&dataset, "MATLAB_int_decode", 1)?;
            }
            Value::Double(value) => {
                let dataset = self.write_dataset(
                    parent,
                    name,
                    &Shape::new([1, 1]).expect("scalar shape"),
                    &[*value],
                    false,
                )?;
                write_class(&dataset, "double")?;
            }
            Value::Complex(value) => {
                let dataset = self.write_dataset(
                    parent,
                    name,
                    &Shape::new([1, 1]).expect("scalar shape"),
                    &[CompoundF64 {
                        real: value.real,
                        imag: value.imaginary,
                    }],
                    false,
                )?;
                write_class(&dataset, "double")?;
            }
            Value::Array(value) => self.write_array(parent, name, value)?,
            Value::Sparse(value) => self.write_sparse(parent, name, value)?,
            Value::Cell(cell) => self.write_cell(parent, name, cell, depth + 1)?,
            Value::Struct(structure) => {
                self.write_struct(parent, name, structure, depth + 1)?;
            }
            Value::String(_) | Value::Table(_) | Value::Object(_) | Value::ObjectArray(_) => {
                return Err(unsupported(format!(
                    "MAT v7.3 persistence for runtime class '{}' is unsupported",
                    value.class_name()
                )));
            }
            Value::Nothing | Value::Graphics(_) | Value::GraphicsArray(_) | Value::Function(_) => {
                return Err(invalid_value(format!(
                    "runtime value of class '{}' is not serializable as MAT v7.3",
                    value.class_name()
                )));
            }
        }
        Ok(())
    }

    fn write_sparse(
        &mut self,
        parent: &Group,
        name: &str,
        value: &SparseArrayData,
    ) -> Result<(), MatError> {
        self.check_sparse_shape(value.shape())?;
        self.account_object()?;
        let group = parent.create_group(name).map_err(|error| {
            hdf5_error(&format!("cannot create MAT sparse group '{name}'"), error)
        })?;
        write_class(&group, value.class_name())?;
        write_u64_attr(&group, "MATLAB_sparse", value.shape().extent(0))?;
        if matches!(value, SparseArrayData::Logical(_)) {
            write_i32_attr(&group, "MATLAB_int_decode", 1)?;
        }

        match value {
            SparseArrayData::Logical(matrix) => {
                self.write_vector_dataset(&group, "jc", matrix.col_offsets(), true)?;
                if matrix.nnz() != 0 {
                    self.write_vector_dataset(&group, "ir", matrix.row_indices(), true)?;
                    let values =
                        self.stage_values(matrix.values(), "sparse logical staging", |value| {
                            u8::from(value.get())
                        })?;
                    self.write_vector_dataset(&group, "data", &values, true)?;
                }
            }
            SparseArrayData::F64(matrix) => {
                self.write_vector_dataset(&group, "jc", matrix.col_offsets(), true)?;
                if matrix.nnz() != 0 {
                    self.write_vector_dataset(&group, "ir", matrix.row_indices(), true)?;
                    self.write_vector_dataset(&group, "data", matrix.values(), true)?;
                }
            }
            SparseArrayData::ComplexF64(matrix) => {
                self.write_vector_dataset(&group, "jc", matrix.col_offsets(), true)?;
                if matrix.nnz() != 0 {
                    self.write_vector_dataset(&group, "ir", matrix.row_indices(), true)?;
                    let values =
                        self.stage_values(matrix.values(), "complex sparse staging", |value| {
                            CompoundF64 {
                                real: value.re,
                                imag: value.im,
                            }
                        })?;
                    self.write_vector_dataset(&group, "data", &values, true)?;
                }
            }
        }
        Ok(())
    }

    fn check_sparse_shape(&self, shape: &Shape) -> Result<(), MatError> {
        if shape.ndims() != 2 {
            return Err(invalid_value(
                "MAT v7.3 sparse arrays must be two-dimensional",
            ));
        }
        let elements = usize::try_from(shape.numel())
            .map_err(|_| limit("MAT v7.3 sparse dense element count is not host-representable"))?;
        if elements > self.limits.maximum_array_elements {
            return Err(limit(format!(
                "MAT v7.3 sparse array has {elements} dense elements, exceeding the configured {}-element limit",
                self.limits.maximum_array_elements
            )));
        }
        Ok(())
    }

    fn write_array(
        &mut self,
        parent: &Group,
        name: &str,
        value: &ArrayData,
    ) -> Result<(), MatError> {
        let (dataset, class, decode) = match value {
            ArrayData::F32(array) => (
                self.write_dataset(parent, name, array.shape(), array.as_slice(), true)?,
                "single",
                None,
            ),
            ArrayData::ComplexF32(array) => {
                let values =
                    self.stage_values(array.as_slice(), "complex single staging", |value| {
                        CompoundF32 {
                            real: value.re,
                            imag: value.im,
                        }
                    })?;
                (
                    self.write_dataset(parent, name, array.shape(), &values, true)?,
                    "single",
                    None,
                )
            }
            ArrayData::F64(array) => (
                self.write_dataset(parent, name, array.shape(), array.as_slice(), true)?,
                "double",
                None,
            ),
            ArrayData::ComplexF64(array) => {
                let values =
                    self.stage_values(array.as_slice(), "complex double staging", |value| {
                        CompoundF64 {
                            real: value.re,
                            imag: value.im,
                        }
                    })?;
                (
                    self.write_dataset(parent, name, array.shape(), &values, true)?,
                    "double",
                    None,
                )
            }
            ArrayData::Logical(array) => {
                let values = self.stage_values(array.as_slice(), "logical staging", |value| {
                    u8::from(value.get())
                })?;
                (
                    self.write_dataset(parent, name, array.shape(), &values, true)?,
                    "logical",
                    Some(1),
                )
            }
            ArrayData::Char(array) => {
                let values =
                    self.stage_values(array.as_slice(), "char staging", |value| value.get())?;
                (
                    self.write_dataset(parent, name, array.shape(), &values, true)?,
                    "char",
                    Some(2),
                )
            }
            ArrayData::Integer(integer) => {
                let dataset = match integer {
                    IntegerArrayData::I8(_) | IntegerArrayData::ComplexI8(_) => {
                        i8::encode(self, parent, name, integer)?
                    }
                    IntegerArrayData::U8(_) | IntegerArrayData::ComplexU8(_) => {
                        u8::encode(self, parent, name, integer)?
                    }
                    IntegerArrayData::I16(_) | IntegerArrayData::ComplexI16(_) => {
                        i16::encode(self, parent, name, integer)?
                    }
                    IntegerArrayData::U16(_) | IntegerArrayData::ComplexU16(_) => {
                        u16::encode(self, parent, name, integer)?
                    }
                    IntegerArrayData::I32(_) | IntegerArrayData::ComplexI32(_) => {
                        i32::encode(self, parent, name, integer)?
                    }
                    IntegerArrayData::U32(_) | IntegerArrayData::ComplexU32(_) => {
                        u32::encode(self, parent, name, integer)?
                    }
                    IntegerArrayData::I64(_) | IntegerArrayData::ComplexI64(_) => {
                        i64::encode(self, parent, name, integer)?
                    }
                    IntegerArrayData::U64(_) | IntegerArrayData::ComplexU64(_) => {
                        u64::encode(self, parent, name, integer)?
                    }
                };
                (dataset, integer.class_name(), None)
            }
        };
        write_class(&dataset, class)?;
        if let Some(value) = decode {
            write_i32_attr(&dataset, "MATLAB_int_decode", value)?;
        }
        Ok(())
    }

    fn write_cell(
        &mut self,
        parent: &Group,
        name: &str,
        cell: &CellArray,
        depth: usize,
    ) -> Result<(), MatError> {
        let references = self.write_references(cell.values(), depth)?;
        let dataset = self.write_dataset(parent, name, cell.shape(), &references, false)?;
        write_class(&dataset, "cell")
    }

    fn write_struct(
        &mut self,
        parent: &Group,
        name: &str,
        structure: &StructArray,
        depth: usize,
    ) -> Result<(), MatError> {
        self.account_object()?;
        let group = parent.create_group(name).map_err(|error| {
            hdf5_error(&format!("cannot create MAT struct group '{name}'"), error)
        })?;
        write_class(&group, "struct")?;
        self.write_fields(&group, structure.field_names())?;
        for (index, field) in structure.field_names().iter().enumerate() {
            validate_hdf5_name(field.as_str(), "MAT struct field")?;
            let values = structure
                .field_values(index)
                .ok_or_else(|| invalid_value("MAT struct field column is missing"))?;
            let references = self.write_references(values, depth)?;
            self.write_dataset(
                &group,
                field.as_str(),
                structure.shape(),
                &references,
                false,
            )?;
        }
        Ok(())
    }

    fn write_fields(&mut self, location: &Location, fields: &[FieldName]) -> Result<(), MatError> {
        let character_bytes = fields.iter().try_fold(0usize, |total, field| {
            total
                .checked_add(field.as_str().len())
                .ok_or_else(|| limit("MATLAB_fields character byte count overflow"))
        })?;
        self.account_staging::<u8>(character_bytes, "MATLAB_fields characters")?;
        self.account_staging::<VarLenArray<FixedAscii<1>>>(fields.len(), "MATLAB_fields table")?;
        write_fields(location, fields)
    }

    fn write_references(
        &mut self,
        values: &[Value],
        depth: usize,
    ) -> Result<Vec<ObjectReference1>, MatError> {
        self.reference_count = self
            .reference_count
            .checked_add(values.len())
            .ok_or_else(|| limit("MAT v7.3 reference count overflow"))?;
        if self.reference_count > self.limits.maximum_hdf5_references {
            return Err(limit(format!(
                "MAT v7.3 reference count exceeds the configured {}-reference limit",
                self.limits.maximum_hdf5_references
            )));
        }
        self.account_staging::<ObjectReference1>(values.len(), "reference staging")?;
        let mut references = Vec::new();
        references
            .try_reserve_exact(values.len())
            .map_err(|_| limit("MAT v7.3 reference staging allocation failed"))?;
        for (index, value) in values.iter().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            let name = reference_name(self.next_reference);
            self.next_reference = self
                .next_reference
                .checked_add(1)
                .ok_or_else(|| limit("MAT v7.3 reference name sequence overflow"))?;
            let refs = self.refs.clone();
            self.write_value(&refs, &name, value, depth)?;
            references.push(
                self.refs
                    .reference::<ObjectReference1>(&name)
                    .map_err(|error| {
                        hdf5_error("cannot create MAT v7.3 object reference", error)
                    })?,
            );
        }
        Ok(references)
    }

    fn write_empty(
        &mut self,
        parent: &Group,
        name: &str,
        value: &Value,
    ) -> Result<Dataset, MatError> {
        let dimensions = value
            .dimensions()
            .ok_or_else(|| invalid_value("empty MAT value has no array shape"))?;
        let dataset = self.write_raw_shape_dataset(parent, name, dimensions)?;
        write_class(&dataset, value.class_name())?;
        write_u8_attr(&dataset, "MATLAB_empty", 1)?;
        if matches!(value, Value::Array(ArrayData::Logical(_))) {
            write_i32_attr(&dataset, "MATLAB_int_decode", 1)?;
        } else if matches!(value, Value::Array(ArrayData::Char(_))) {
            write_i32_attr(&dataset, "MATLAB_int_decode", 2)?;
        }
        Ok(dataset)
    }

    fn write_raw_shape_dataset(
        &mut self,
        parent: &Group,
        name: &str,
        dimensions: &[u64],
    ) -> Result<Dataset, MatError> {
        self.account(dimensions.len().saturating_mul(size_of::<u64>()))?;
        self.account_object()?;
        let dataset = parent
            .new_dataset::<u64>()
            .shape([dimensions.len()])
            .create(name)
            .map_err(|error| hdf5_error("cannot create MAT empty-shape dataset", error))?;
        dataset
            .write_raw(dimensions)
            .map_err(|error| hdf5_error("cannot write MAT empty-shape dataset", error))?;
        Ok(dataset)
    }

    fn write_vector_dataset<T: H5Type>(
        &mut self,
        parent: &Group,
        name: &str,
        values: &[T],
        allow_compression: bool,
    ) -> Result<Dataset, MatError> {
        let bytes = values
            .len()
            .checked_mul(size_of::<T>())
            .ok_or_else(|| limit("MAT v7.3 vector dataset byte count overflow"))?;
        self.account(bytes)?;
        self.account_object()?;
        let builder = parent.new_dataset::<T>().shape([values.len()]);
        let builder = if allow_compression && bytes >= COMPRESSION_THRESHOLD_BYTES {
            builder.deflate(3)
        } else {
            builder
        };
        let dataset = builder
            .create(name)
            .map_err(|error| hdf5_error(&format!("cannot create MAT dataset '{name}'"), error))?;
        check_cancelled(self.cancellation)?;
        dataset
            .write_raw(values)
            .map_err(|error| hdf5_error(&format!("cannot write MAT dataset '{name}'"), error))?;
        check_cancelled(self.cancellation)?;
        Ok(dataset)
    }

    fn write_dataset<T: H5Type>(
        &mut self,
        parent: &Group,
        name: &str,
        shape: &Shape,
        values: &[T],
        allow_compression: bool,
    ) -> Result<Dataset, MatError> {
        let expected = usize::try_from(shape.numel())
            .map_err(|_| limit("MAT v7.3 array element count is not host-representable"))?;
        if expected != values.len() {
            return Err(invalid_value(format!(
                "MAT v7.3 value buffer has {} elements for a {expected}-element shape",
                values.len()
            )));
        }
        if expected > self.limits.maximum_array_elements {
            return Err(limit(format!(
                "MAT v7.3 array has {expected} elements, exceeding the configured {}-element limit",
                self.limits.maximum_array_elements
            )));
        }
        let bytes = expected
            .checked_mul(size_of::<T>())
            .ok_or_else(|| limit("MAT v7.3 dataset byte count overflow"))?;
        self.account(bytes)?;
        self.account_object()?;
        let raw_shape = raw_shape(shape)?;
        let builder = parent.new_dataset::<T>().shape(raw_shape);
        let builder = if allow_compression && bytes >= COMPRESSION_THRESHOLD_BYTES {
            builder.deflate(3)
        } else {
            builder
        };
        let dataset = builder
            .create(name)
            .map_err(|error| hdf5_error(&format!("cannot create MAT dataset '{name}'"), error))?;
        check_cancelled(self.cancellation)?;
        dataset
            .write_raw(values)
            .map_err(|error| hdf5_error(&format!("cannot write MAT dataset '{name}'"), error))?;
        check_cancelled(self.cancellation)?;
        Ok(dataset)
    }

    fn account(&mut self, bytes: usize) -> Result<(), MatError> {
        self.estimated_bytes = self
            .estimated_bytes
            .checked_add(bytes)
            .and_then(|value| value.checked_add(1_024))
            .ok_or_else(|| limit("MAT v7.3 output size estimate overflow"))?;
        if self.estimated_bytes > self.limits.maximum_allocation_bytes {
            return Err(limit(format!(
                "MAT v7.3 estimated allocation exceeds the configured {}-byte limit",
                self.limits.maximum_allocation_bytes
            )));
        }
        Ok(())
    }

    fn account_staging<T>(&mut self, length: usize, purpose: &str) -> Result<(), MatError> {
        let bytes = length
            .checked_mul(size_of::<T>())
            .ok_or_else(|| limit(format!("{purpose} byte count overflow")))?;
        self.estimated_bytes = self
            .estimated_bytes
            .checked_add(bytes)
            .ok_or_else(|| limit(format!("{purpose} allocation count overflow")))?;
        if self.estimated_bytes > self.limits.maximum_allocation_bytes {
            return Err(limit(format!(
                "MAT v7.3 {purpose} exceeds the configured {}-byte allocation limit",
                self.limits.maximum_allocation_bytes
            )));
        }
        Ok(())
    }

    fn stage_values<T, U>(
        &mut self,
        input: &[T],
        purpose: &str,
        mut convert: impl FnMut(&T) -> U,
    ) -> Result<Vec<U>, MatError> {
        self.account_staging::<U>(input.len(), purpose)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(input.len())
            .map_err(|_| limit(format!("{purpose} allocation failed")))?;
        for (index, value) in input.iter().enumerate() {
            if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                check_cancelled(self.cancellation)?;
            }
            values.push(convert(value));
        }
        Ok(values)
    }

    fn account_object(&mut self) -> Result<(), MatError> {
        self.object_count = self
            .object_count
            .checked_add(1)
            .ok_or_else(|| limit("MAT v7.3 output object count overflow"))?;
        if self.object_count > self.limits.maximum_hdf5_objects {
            return Err(limit(format!(
                "MAT v7.3 output object count exceeds the configured {}-object limit",
                self.limits.maximum_hdf5_objects
            )));
        }
        Ok(())
    }

    fn check_depth(&self, depth: usize) -> Result<(), MatError> {
        if depth > self.limits.maximum_recursion_depth {
            Err(limit(format!(
                "MAT v7.3 nesting exceeds the configured {}-level limit",
                self.limits.maximum_recursion_depth
            )))
        } else {
            Ok(())
        }
    }
}

fn raw_shape(shape: &Shape) -> Result<Vec<usize>, MatError> {
    shape
        .dimensions()
        .iter()
        .rev()
        .map(|dimension| {
            usize::try_from(*dimension)
                .map_err(|_| limit("MAT v7.3 dimension is not host-representable"))
        })
        .collect()
}

fn should_use_empty_representation(value: &Value) -> bool {
    !matches!(value, Value::Sparse(_))
        && value.dimensions().is_some_and(|dimensions| {
            dimensions.contains(&0)
                || matches!(value, Value::Struct(structure) if structure.field_count() == 0)
        })
}

fn reference_name(mut index: u64) -> String {
    let mut bytes = Vec::new();
    loop {
        let digit = u8::try_from(index % 26).expect("base-26 digit fits u8");
        bytes.push(b'a' + digit);
        index /= 26;
        if index == 0 {
            break;
        }
        index -= 1;
    }
    bytes.reverse();
    String::from_utf8(bytes).expect("base-26 reference names are ASCII")
}

fn write_class(location: &Location, class: &str) -> Result<(), MatError> {
    macro_rules! fixed {
        ($length:literal) => {{
            let value = FixedAscii::<$length>::from_ascii(class).map_err(|error| {
                invalid_value(format!("invalid MATLAB class attribute: {error}"))
            })?;
            location
                .new_attr::<FixedAscii<$length>>()
                .create("MATLAB_class")
                .and_then(|attribute| attribute.write_scalar(&value))
                .map_err(|error| hdf5_error("cannot write MATLAB_class attribute", error))
        }};
    }
    match class.len() {
        4 => fixed!(4),
        5 => fixed!(5),
        6 => fixed!(6),
        7 => fixed!(7),
        15 => fixed!(15),
        _ => Err(invalid_value(format!(
            "unsupported MATLAB class attribute '{class}'"
        ))),
    }
}

fn write_fields(location: &Location, fields: &[FieldName]) -> Result<(), MatError> {
    if fields.is_empty() {
        return Ok(());
    }
    let mut raw = Vec::new();
    raw.try_reserve_exact(fields.len())
        .map_err(|_| limit("MATLAB_fields staging allocation failed"))?;
    for field in fields {
        validate_hdf5_name(field.as_str(), "MAT struct field")?;
        if !field.as_str().is_ascii() {
            return Err(invalid_value(format!(
                "MAT v7.3 struct field '{}' is not ASCII",
                field.as_str()
            )));
        }
        let mut characters = Vec::new();
        characters
            .try_reserve_exact(field.as_str().len())
            .map_err(|_| limit("MATLAB_fields character allocation failed"))?;
        for byte in field.as_str().as_bytes() {
            characters.push(
                FixedAscii::<1>::from_ascii(&[*byte])
                    .expect("validated ASCII field-name byte fits fixed string"),
            );
        }
        raw.push(VarLenArray::from_slice(&characters));
    }
    location
        .new_attr_builder()
        .with_data(&raw)
        .create("MATLAB_fields")
        .map(|_| ())
        .map_err(|error| hdf5_error("cannot write MATLAB_fields attribute", error))
}

fn write_u8_attr(location: &Location, name: &str, value: u8) -> Result<(), MatError> {
    location
        .new_attr::<u8>()
        .create(name)
        .and_then(|attribute| attribute.write_scalar(&value))
        .map_err(|error| hdf5_error(&format!("cannot write {name} attribute"), error))
}

fn write_i32_attr(location: &Location, name: &str, value: i32) -> Result<(), MatError> {
    location
        .new_attr::<i32>()
        .create(name)
        .and_then(|attribute| attribute.write_scalar(&value))
        .map_err(|error| hdf5_error(&format!("cannot write {name} attribute"), error))
}

fn write_u64_attr(location: &Location, name: &str, value: u64) -> Result<(), MatError> {
    location
        .new_attr::<u64>()
        .create(name)
        .and_then(|attribute| attribute.write_scalar(&value))
        .map_err(|error| hdf5_error(&format!("cannot write {name} attribute"), error))
}

#[allow(clippy::needless_pass_by_value)]
fn variable_error(name: &str, error: MatError) -> MatError {
    MatError::with_kind(error.kind(), format!("variable '{name}': {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_hdf5_version_is_locked() {
        assert_eq!(hdf5::library_version(), (2, 2, 0));
    }

    fn cyclic_cell_image() -> Vec<u8> {
        let encoder = Encoder::new(MatLimits::default(), None).unwrap();
        let nested = encoder
            .refs
            .new_dataset::<ObjectReference1>()
            .shape([1, 1])
            .create("a")
            .unwrap();
        write_class(&nested, "cell").unwrap();
        let self_reference = encoder.refs.reference::<ObjectReference1>("a").unwrap();
        nested.write_raw(&[self_reference]).unwrap();

        let root = encoder
            .root
            .new_dataset::<ObjectReference1>()
            .shape([1, 1])
            .create("cycle")
            .unwrap();
        write_class(&root, "cell").unwrap();
        let nested_reference = encoder.refs.reference::<ObjectReference1>("a").unwrap();
        root.write_raw(&[nested_reference]).unwrap();
        let limits = MatLimits::default();
        extract_image(
            &encoder.file,
            limits.maximum_file_bytes,
            limits.maximum_allocation_bytes,
        )
        .unwrap()
    }

    fn linked_refs_image(hard: bool) -> Vec<u8> {
        let encoder = Encoder::new(MatLimits::default(), None).unwrap();
        if hard {
            encoder.root.link_hard("/#refs#", "alias").unwrap();
        } else {
            encoder.root.link_soft("/#refs#", "alias").unwrap();
        }
        let limits = MatLimits::default();
        extract_image(
            &encoder.file,
            limits.maximum_file_bytes,
            limits.maximum_allocation_bytes,
        )
        .unwrap()
    }

    fn invalid_sparse_image() -> Vec<u8> {
        let encoder = Encoder::new(MatLimits::default(), None).unwrap();
        let sparse = encoder.root.create_group("bad").unwrap();
        write_class(&sparse, "double").unwrap();
        write_u64_attr(&sparse, "MATLAB_sparse", 2).unwrap();
        sparse
            .new_dataset_builder()
            .with_data(&[0_u64, 1, 2])
            .create("jc")
            .unwrap();
        sparse
            .new_dataset_builder()
            .with_data(&[0_u64, 2])
            .create("ir")
            .unwrap();
        sparse
            .new_dataset_builder()
            .with_data(&[1.0_f64, 2.0])
            .create("data")
            .unwrap();
        let limits = MatLimits::default();
        extract_image(
            &encoder.file,
            limits.maximum_file_bytes,
            limits.maximum_allocation_bytes,
        )
        .unwrap()
    }

    #[test]
    fn reference_names_follow_matlab_base_26_style() {
        assert_eq!(reference_name(0), "a");
        assert_eq!(reference_name(25), "z");
        assert_eq!(reference_name(26), "aa");
        assert_eq!(reference_name(27), "ab");
        assert_eq!(reference_name(701), "zz");
        assert_eq!(reference_name(702), "aaa");
    }

    #[test]
    fn cyclic_refs_are_rejected_before_recursion_can_repeat() {
        let error = OfficialHdf5Provider
            .decode(&cyclic_cell_image(), MatLimits::default(), None)
            .unwrap_err();
        assert_eq!(error.kind(), MatErrorKind::InvalidFormat);
        assert!(error.message().contains("reference cycle"));
        assert!(error.message().contains("/#refs#/a"));
    }

    #[test]
    fn soft_links_and_hard_link_aliases_are_rejected() {
        let soft = OfficialHdf5Provider
            .decode(&linked_refs_image(false), MatLimits::default(), None)
            .unwrap_err();
        assert_eq!(soft.kind(), MatErrorKind::Unsupported);
        assert!(soft.message().contains("forbidden Soft HDF5 link"));

        let hard = OfficialHdf5Provider
            .decode(&linked_refs_image(true), MatLimits::default(), None)
            .unwrap_err();
        assert_eq!(hard.kind(), MatErrorKind::Unsupported);
        assert!(hard.message().contains("hard links"));
    }

    #[test]
    fn sparse_csc_indices_are_checked_before_construction() {
        let error = OfficialHdf5Provider
            .decode(&invalid_sparse_image(), MatLimits::default(), None)
            .unwrap_err();
        assert_eq!(error.kind(), MatErrorKind::InvalidFormat);
        assert!(error.message().contains("sparse CSC"));
    }
}
