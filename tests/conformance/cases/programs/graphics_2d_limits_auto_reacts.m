figure_handle = figure();
axes_handle = gca();
line_handle = plot(axes_handle, [0, 10], [2, 4]);
xlim(axes_handle, 'auto');
ylim(axes_handle, 'auto');

initial_limits = [xlim(axes_handle), ylim(axes_handle)];
initial_modes = {get(axes_handle, 'XLimMode'), get(axes_handle, 'YLimMode')};
set(line_handle, 'XData', [100, 200], 'YData', [-10, 30]);
changed_limits = [xlim(axes_handle), ylim(axes_handle)];
changed_modes = {get(axes_handle, 'XLimMode'), get(axes_handle, 'YLimMode')};

openmat_result = {initial_limits, initial_modes, changed_limits, changed_modes};
close(figure_handle);
