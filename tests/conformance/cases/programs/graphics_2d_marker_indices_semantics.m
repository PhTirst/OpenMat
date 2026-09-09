figure_handle = figure();
line_handle = plot(1:6, [2, 4, 3, 5, 4, 6], 'o');

default_indices = get(line_handle, 'MarkerIndices');
set(line_handle, 'MarkerIndices', [5; 2; 9; 5]);
explicit_indices = get(line_handle, 'MarkerIndices');
set(line_handle, 'XData', 1:2, 'YData', [8, 9]);
preserved_indices = get(line_handle, 'MarkerIndices');
set(line_handle, 'MarkerIndices', []);
empty_indices = get(line_handle, 'MarkerIndices');

openmat_result = { ...
    class(default_indices), size(default_indices), default_indices, ...
    size(explicit_indices), explicit_indices, preserved_indices, ...
    class(empty_indices), size(empty_indices), empty_indices ...
};
close(figure_handle);
