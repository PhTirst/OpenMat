#![cfg(windows)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use openmat_ffi::{
    Abi, CArray, CStruct, CType, CUnion, FunctionType, Library, Memory, NativeFunction,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "openmat-ffi-native-layout-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).expect("create FFI test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn compile_fixture() -> Option<(TestDirectory, Arc<Library>)> {
    if Command::new("gcc").arg("--version").output().is_err() {
        eprintln!("skipping native FFI layout fixture because gcc is unavailable");
        return None;
    }
    let directory = TestDirectory::new();
    let library_path = directory.path().join("basic_ffi.dll");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic_ffi.c");
    let status = Command::new("gcc")
        .arg("-shared")
        .arg("-std=c11")
        .arg("-O2")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg(fixture)
        .arg("-o")
        .arg(&library_path)
        .status()
        .expect("run gcc for FFI fixture");
    assert!(status.success(), "C FFI fixture must compile cleanly");
    let library = Library::open(library_path).expect("load compiled FFI fixture");
    Some((directory, library))
}

fn function(
    library: &Arc<Library>,
    name: &str,
    arguments: Vec<Arc<CType>>,
    result: Arc<CType>,
) -> Arc<NativeFunction> {
    NativeFunction::new(
        library
            .symbol_address(name)
            .expect("resolve fixture symbol"),
        Some(Arc::clone(library)),
        FunctionType {
            abi: Abi::Win64,
            arguments,
            result,
        },
    )
}

fn pointer_argument(address: usize, pointee: Arc<CType>) -> Memory {
    let pointer_type = CType::Pointer(pointee);
    let memory = Memory::allocate(&pointer_type).unwrap();
    memory.write(&address.to_ne_bytes());
    memory
}

fn u32_result(function: &NativeFunction, arguments: &[Memory]) -> u32 {
    let result = function.call(arguments).unwrap().unwrap().to_vec();
    u32::from_ne_bytes(result[..4].try_into().unwrap())
}

fn check_packed_layout(library: &Arc<Library>) {
    let packed = Arc::new(
        CStruct::with_pack(
            "openmat_ffi_packed",
            vec![
                ("tag".to_owned(), Arc::new(CType::U8)),
                ("value".to_owned(), Arc::new(CType::U32)),
            ],
            Some(1),
        )
        .unwrap(),
    );
    let packed_type = Arc::new(CType::Structure(Arc::clone(&packed)));
    assert_eq!(
        u32_result(
            &function(
                library,
                "ffi_sizeof_packed",
                Vec::new(),
                Arc::new(CType::U32)
            ),
            &[],
        ),
        u32::try_from(packed.size).unwrap()
    );
    assert_eq!(
        u32_result(
            &function(
                library,
                "ffi_offsetof_packed_value",
                Vec::new(),
                Arc::new(CType::U32),
            ),
            &[],
        ),
        u32::try_from(packed.fields[1].offset).unwrap()
    );
    let packed_memory = Memory::allocate(&packed_type).unwrap();
    let tag = Memory::allocate(&CType::U8).unwrap();
    tag.write(&[0x2a]);
    let payload = Memory::allocate(&CType::U32).unwrap();
    payload.write(&0x1234_5678_u32.to_ne_bytes());
    function(
        library,
        "ffi_write_packed",
        vec![
            Arc::new(CType::Pointer(Arc::clone(&packed_type))),
            Arc::new(CType::U8),
            Arc::new(CType::U32),
        ],
        Arc::new(CType::Void),
    )
    .call(&[
        pointer_argument(packed_memory.address(), Arc::clone(&packed_type)),
        tag,
        payload,
    ])
    .unwrap();
    assert_eq!(packed_memory.to_vec(), [0x2a, 0x78, 0x56, 0x34, 0x12]);
    assert_eq!(
        u32_result(
            &function(
                library,
                "ffi_read_packed",
                vec![Arc::clone(&packed_type)],
                Arc::new(CType::U32),
            ),
            &[packed_memory],
        ),
        0x1234_56a2
    );
}

fn check_union_layout(library: &Arc<Library>) {
    let union = Arc::new(
        CUnion::new(
            "openmat_ffi_number",
            vec![
                ("bits".to_owned(), Arc::new(CType::U32)),
                ("real".to_owned(), Arc::new(CType::F32)),
            ],
        )
        .unwrap(),
    );
    let union_type = Arc::new(CType::Union(union));
    let union_memory = Memory::allocate(&union_type).unwrap();
    union_memory.write(&0x3f80_0000_u32.to_ne_bytes());
    assert_eq!(
        u32_result(
            &function(
                library,
                "ffi_read_union_bits",
                vec![Arc::clone(&union_type)],
                Arc::new(CType::U32),
            ),
            &[union_memory],
        ),
        0x3f80_0000
    );
}

fn check_array_field_layout(library: &Arc<Library>) {
    let words_array = Arc::new(CArray::new(Arc::new(CType::U16), 3).unwrap());
    let words = Arc::new(
        CStruct::new(
            "openmat_ffi_words",
            vec![(
                "values".to_owned(),
                Arc::new(CType::Array(Arc::clone(&words_array))),
            )],
        )
        .unwrap(),
    );
    let words_type = Arc::new(CType::Structure(words));
    let words_memory = Memory::allocate(&words_type).unwrap();
    let mut words_bytes = Vec::new();
    for value in [11_u16, 17, 23] {
        words_bytes.extend_from_slice(&value.to_ne_bytes());
    }
    words_memory.write(&words_bytes);
    assert_eq!(
        u32_result(
            &function(
                library,
                "ffi_sum_words",
                vec![Arc::clone(&words_type)],
                Arc::new(CType::U32),
            ),
            &[words_memory],
        ),
        51
    );
}

#[test]
fn declared_packing_union_and_array_layout_match_compiled_c() {
    let Some((_directory, library)) = compile_fixture() else {
        return;
    };
    check_packed_layout(&library);
    check_union_layout(&library);
    check_array_field_layout(&library);
}
