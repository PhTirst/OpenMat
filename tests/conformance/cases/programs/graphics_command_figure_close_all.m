close all;

first_figure = figure('Color', 'w', 'Visible', 'off');
second_figure = figure('Color', [0.25 0.5 0.75], 'Visible', 'off');

white_color = isequal(get(first_figure, 'Color'), [1 1 1]);
explicit_color = isequal(get(second_figure, 'Color'), [0.25 0.5 0.75]);
both_live = isgraphics(first_figure, 'figure') && ...
    isgraphics(second_figure, 'figure');
second_is_current = get(groot, 'CurrentFigure') == second_figure;

close all;
command_closed_both = ~isgraphics(first_figure) && ~isgraphics(second_figure);
command_cleared_current = isempty(get(groot, 'CurrentFigure'));

third_figure = figure('Color', 'w', 'Visible', 'off');
close('all');
functional_closed = ~isgraphics(third_figure);
functional_cleared_current = isempty(get(groot, 'CurrentFigure'));

openmat_result = [ ...
    white_color, explicit_color, both_live, second_is_current, ...
    command_closed_both, command_cleared_current, functional_closed, ...
    functional_cleared_current ...
];
