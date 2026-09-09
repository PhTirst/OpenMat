cells = {1, 2};
cells_alias = cells;
cell_failed = false;
try
    cells(1:2) = {3, 4, 5};
catch
    cell_failed = true;
end

records = struct('a', {1, 2});
records_alias = records;
struct_failed = false;
try
    records(1) = struct('b', 9);
catch
    struct_failed = true;
end

openmat_result = {cell_failed, cells, cells_alias, ...
    struct_failed, records, records_alias};
