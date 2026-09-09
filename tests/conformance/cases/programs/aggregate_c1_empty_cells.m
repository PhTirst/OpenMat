empty_literal = {};
shaped_empty = cell(0, 3);
column_major = {11, 22; 33, 44};
heterogeneous = {int16(-7), 'AZ', ["x", missing], true; ...
    complex(single(1), single(-2)), uint64(9), [], false};

openmat_result = {empty_literal, shaped_empty, ...
    column_major, heterogeneous};
