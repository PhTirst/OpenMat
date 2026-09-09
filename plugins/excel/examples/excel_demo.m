% Run with --oex-plugin <path>/openmat_excel.dll.
% The destination should not already exist (or explicitly enable overwrite).
data = [1, 10, 100; 2, 20, 200; 3, NaN, 300];
excel_write('experiment.xlsx', data, 'Measurements', 'B2');

[loaded, kinds, origin] = excel_read('experiment.xlsx', 'Measurements');
disp(loaded);
disp(origin); % [2, 2]: the used rectangle starts at B2.

region = excel_read('experiment.xlsx', 1, 'B2:D3');
disp(region);
for index = 1:excel_sheet_count('experiment.xlsx')
    disp(excel_sheet_name('experiment.xlsx', index));
end
