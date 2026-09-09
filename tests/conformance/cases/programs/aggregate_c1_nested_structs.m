shaped_empty = struct('beta', cell(0, 3), 'alpha', cell(0, 3));

nested = struct();
nested.beta = {uint8(5), {true, 'q'}};
nested.alpha = string(char(uint16([65, 55357])));

openmat_result = {nested, shaped_empty};
