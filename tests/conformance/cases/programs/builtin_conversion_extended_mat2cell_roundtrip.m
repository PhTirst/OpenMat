source = reshape(int16(1:24), [4, 3, 2]);
parts = mat2cell(source, [1, 3], [2, 1], [1, 1]);
openmat_result = cell2mat(parts);
