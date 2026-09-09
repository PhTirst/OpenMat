figure_handle = figure();
axes_handle = gca();
line_handle = plot(axes_handle, [0, 1], [0, 1]);
xlim(axes_handle, [10, 20]);
ylim(axes_handle, [-2, 2]);

before_change = [xlim(axes_handle), ylim(axes_handle)];
before_modes = {get(axes_handle, 'XLimMode'), get(axes_handle, 'YLimMode')};
set(line_handle, 'XData', [100, 200], 'YData', [10, 20]);
after_change = [xlim(axes_handle), ylim(axes_handle)];
after_modes = {get(axes_handle, 'XLimMode'), get(axes_handle, 'YLimMode')};

openmat_result = {before_change, before_modes, after_change, after_modes};
close(figure_handle);
