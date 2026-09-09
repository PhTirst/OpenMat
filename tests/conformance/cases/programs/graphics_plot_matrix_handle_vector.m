figure_handle = figure();
line_handles = plot([1, 2; 3, 4; 5, 6]);
openmat_result = [numel(line_handles), size(line_handles, 1), size(line_handles, 2)];
close(figure_handle);
