use calamine::{Data, Reader, open_workbook_auto};
use openmat_kernel::{ExecutionEngine, Kernel, RuntimeEngine};
use openmat_protocol::{
    ExecuteRequest, ExecutionMode, InitializeRequest, Request, RequestEnvelope, ServerMessage,
};
use rust_xlsxwriter::{Format, Formula, Workbook};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let plugin = args.next().expect("plugin DLL path");
    let script = args.next().expect("integration script path");
    let directory = tempfile::Builder::new()
        .prefix("oex-excel-测试-")
        .tempdir()
        .unwrap();
    let fixture = directory.path().join("fixture.xlsx");
    let mut book = Workbook::new();
    book.add_worksheet().set_name("Empty").unwrap();
    let sheet = book.add_worksheet();
    sheet.set_name("测量").unwrap();
    sheet.write_string(0, 0, "header").unwrap();
    sheet.write_number(1, 0, 7.5).unwrap();
    sheet.write_boolean(1, 1, true).unwrap();
    sheet
        .write_number_with_format(1, 2, 45000.0, &Format::new().set_num_format("yyyy-mm-dd"))
        .unwrap();
    sheet
        .write_formula(1, 3, Formula::new("3*7").set_result("21"))
        .unwrap();
    book.save(fixture).unwrap();

    // SAFETY: this test loads only the explicitly supplied, locally built plugin.
    let engine = unsafe { RuntimeEngine::with_oex_plugins([plugin]) }.expect("load Excel plugin");
    let initialize = Request::Initialize(InitializeRequest {
        client: engine.implementation(),
        supported_protocols: vec![],
        capabilities: engine.capabilities(),
    });
    let code = std::fs::read_to_string(&script).unwrap().replace(
        "@@DIR@@",
        &directory
            .path()
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "''"),
    );
    let executed_script = directory.path().join("integration.m");
    std::fs::write(&executed_script, &code).unwrap();
    let execute = Request::Execute(ExecuteRequest {
        code,
        source_name: executed_script.to_string_lossy().into_owned(),
        mode: ExecutionMode::File,
    });
    let mut kernel = Kernel::new("excel-plugin-test", engine);
    for (i, request) in [initialize, execute].into_iter().enumerate() {
        let messages = kernel.handle_request(&RequestEnvelope::new(
            "excel-plugin-test",
            i.to_string(),
            request,
        ));
        if messages
            .iter()
            .any(|message| matches!(message, ServerMessage::Response(response) if !response.ok))
        {
            panic!("Excel integration failed: {messages:#?}");
        }
    }
    // Independently inspect the workbook generated from m-language calls.
    let mut book = open_workbook_auto(directory.path().join("roundtrip 中文.xlsx")).unwrap();
    assert_eq!(book.sheet_names(), ["结果"]);
    let cells = book.worksheet_range("结果").unwrap();
    assert_eq!(cells.start(), Some((3, 2)));
    assert_eq!(cells.get_value((3, 2)), Some(&Data::Float(9.0)));
    assert_eq!(cells.get_value((4, 3)), Some(&Data::Float(12.0)));
    println!("Excel OEX integration passed (real kernel + DLL + XLSX files)");
}
