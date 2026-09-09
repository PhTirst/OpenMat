a = min([1 4; 5 2], [2 3; 4 6]);
b = min([1; 4], [2 3]);
c = double(max(single([1 4; 5 2]), 3));
d = min([NaN 1], [2 NaN], 'includenan');
openmat_result = [a(:).', b(:).', c(:).', d];
