file = '@@DIR@@/roundtrip 中文.xlsx';
a = [1, 2, 3; 4, NaN, 6];
excel_write(file, a, '结果', 'C4');
[b, kind, origin] = excel_read(file);
assert(isequal(size(b), [2, 3]));
assert(isequal(b(:, [1, 3]), a(:, [1, 3])));
assert(b(1, 2) == 2 && isnan(b(2, 2)));
assert(isequal(kind, uint8([1, 1, 1; 1, 0, 1])));
assert(isequal(origin, [4, 3]));
assert(excel_sheet_count(file) == 1);
assert(strcmp(excel_sheet_name(file, 1), '结果'));
c = excel_read(file, '结果', '$D$4:E5');
assert(isequal(size(c), [2, 2]) && c(1, 1) == 2 && c(2, 2) == 6);
blank = excel_read(file, 1, 'A1:B2');
assert(all(isnan(blank(:))));

failed = false;
try
    excel_write(file, [99, 100]);
catch err
    failed = strcmp(err.identifier, 'OpenMat:Excel:Exists');
end
assert(failed);
excel_write(file, [9, 11; 10, 12], '结果', 'C4', true);
assert(isequal(excel_read(file), [9, 11; 10, 12]));
failed = false;
try
    excel_write(file, Inf, '结果', 'A1', true);
catch err
    failed = strcmp(err.identifier, 'OpenMat:Excel:NonFinite');
end
assert(failed && isequal(excel_read(file), [9, 11; 10, 12]));

fixture = '@@DIR@@/fixture.xlsx';
assert(excel_sheet_count(fixture) == 2);
assert(strcmp(excel_sheet_name(fixture, 2), '测量'));
empty = excel_read(fixture);
assert(isempty(empty));
[m, k] = excel_read(fixture, 2, 'A1:D2');
assert(m(2, 1) == 7.5 && m(2, 2) == 1 && m(2, 4) == 21);
assert(isnan(m(1, 1)) && isnan(m(2, 3)));
assert(isequal(k, uint8([3, 0, 0, 0; 1, 2, 4, 1])));

excel_write('@@DIR@@/logical.xlsx', logical([1, 0]));
[m, k] = excel_read('@@DIR@@/logical.xlsx');
assert(isequal(m, [1, 0]) && isequal(k, uint8([2, 2])));
excel_write('@@DIR@@/integer.xlsx', int32([-3, 7]));
assert(isequal(excel_read('@@DIR@@/integer.xlsx'), [-3, 7]));
excel_write('@@DIR@@/single.xlsx', single([1.5, 2.5]));
assert(isequal(excel_read('@@DIR@@/single.xlsx'), [1.5, 2.5]));

expect_excel_error(@() excel_read(fixture, 0), 'OpenMat:Excel:Sheet');
expect_excel_error(@() excel_read(fixture, 'missing'), 'OpenMat:Excel:Sheet');
expect_excel_error(@() excel_read(fixture, 1, 'B2:A1'), 'OpenMat:Excel:Range');
expect_excel_error(@() excel_read(fixture, 1, 'A1:XFD1048576'), 'OpenMat:Excel:Size');
expect_excel_error(@() excel_read('@@DIR@@/missing.xlsx'), 'OpenMat:Excel:IO');
expect_excel_error(@() excel_read("string-not-char.xlsx"), 'OpenMat:Excel:Argument');

function expect_excel_error(f, identifier)
failed = false;
try
    ignored = f();
catch err
    failed = strcmp(err.identifier, identifier);
end
assert(failed);
end
