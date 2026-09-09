source = [1, 2, 3; 4, 5, 6];
column_totals = zeros(1, 3);
column_index = 0;
for column = source
    column_index = column_index + 1;
    column_totals(column_index) = sum(column);
end
openmat_result = column_totals;
