figure_handle = figure();
axes_handle = gca();
first_line = plot([1, 2, 3]);
hold(axes_handle, 'on');
second_line = plot([4, 5, 6]);
openmat_result = numel(get(axes_handle, 'Children'));
close(figure_handle);
