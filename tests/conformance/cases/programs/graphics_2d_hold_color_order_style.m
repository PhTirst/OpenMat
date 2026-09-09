figure_handle = figure();
axes_handle = gca();
hold(axes_handle, 'on');
set(axes_handle, 'ColorOrder', [1, 0, 0; 0, 0.5, 0]);
set(axes_handle, 'LineStyleOrder', {'--'});

initial_color_index = get(axes_handle, 'ColorOrderIndex');
first_line = plot(axes_handle, 1:3, [1, 2, 3]);
after_first_color_index = get(axes_handle, 'ColorOrderIndex');
second_line = plot(axes_handle, 1:3, [3, 2, 1]);
after_second_color_index = get(axes_handle, 'ColorOrderIndex');

openmat_result = { ...
    [initial_color_index, after_first_color_index, after_second_color_index], ...
    get(first_line, 'Color'), ...
    get(second_line, 'Color'), ...
    get(first_line, 'LineStyle'), ...
    get(second_line, 'LineStyle') ...
};
close(figure_handle);
