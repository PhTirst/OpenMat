[x, y] = meshgrid(linspace(-4, 4, 100));
z = sin(sqrt(x.^2 + y.^2));
figure_handle = figure();
surf(x, y, z);
colorbar_handle = colorbar();

automatic_before = get(colorbar_handle, 'Ticks');
set(colorbar_handle, 'Ticks', [-0.5, 0, 0.5]);
manual_ticks = get(colorbar_handle, 'Ticks');
set(colorbar_handle, 'TicksMode', 'auto');
automatic_after = get(colorbar_handle, 'Ticks');

openmat_result = [automatic_before, manual_ticks, automatic_after];
close(figure_handle);
