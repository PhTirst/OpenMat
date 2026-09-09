figure_handle = figure();
axes_handle = gca();
line_handle = plot([1, 2, 3]);
hold(axes_handle, 'on');
cla(axes_handle);
openmat_result = ishold(axes_handle);
close(figure_handle);
