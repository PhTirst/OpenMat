unfielded_scalar = struct();
unfielded_empty = struct([]);
ordered_shaped_empty = struct( ...
    'beta', cell(0, 3), 'alpha', cell(0, 3));

openmat_result = {unfielded_scalar, ...
    unfielded_empty, ordered_shaped_empty};
