figure_handle = figure('Visible', 'off');
old_axes = axes(figure_handle);

layout = tiledlayout(figure_handle, 2, 2);
first_axes = nexttile(layout);
selected_first_axes = nexttile(layout, 1);
second_axes = nexttile(layout);

first_position = get(first_axes, 'Position');
second_position = get(second_axes, 'Position');
grid_size = get(layout, 'GridSize');
same_tile = isequal(first_axes, selected_first_axes);
parent_matches = isequal(get(first_axes, 'Parent'), layout);
layout_child_count = numel(get(layout, 'Children'));
figure_child_count = numel(get(figure_handle, 'Children'));
old_axes_valid = isgraphics(old_axes);

subplot_axes = subplot(1, 1, 1);
layout_valid_after_subplot = isgraphics(layout);
first_valid_after_subplot = isgraphics(first_axes);
second_valid_after_subplot = isgraphics(second_axes);
subplot_valid = isgraphics(subplot_axes);
subplot_child_count = numel(get(figure_handle, 'Children'));

openmat_result = [ ...
    first_position, second_position, grid_size, ...
    same_tile, parent_matches, layout_child_count, figure_child_count, old_axes_valid, ...
    layout_valid_after_subplot, first_valid_after_subplot, second_valid_after_subplot, ...
    subplot_valid, subplot_child_count ...
];

close(figure_handle);
