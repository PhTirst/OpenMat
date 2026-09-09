rng(7);
n = 500;

x = randn(n,1);
y = randn(n,1);
z = randn(n,1);
c = sqrt(x.^2 + y.^2 + z.^2);

figure_handle = figure();
scatter_handle = scatter3(x, y, z, 40, c, 'filled');
xlabel('X');
ylabel('Y');
zlabel('Z');
title('3D Scatter');
colorbar;
grid on;

axes_handle = gca();
scatter_cdata = get(scatter_handle, 'CData');
color_limits = get(axes_handle, 'CLim');
openmat_result = [ ...
    size(x), size(y), size(z), size(c), ...
    size(scatter_cdata), ...
    get(scatter_handle, 'SizeData'), ...
    double(color_limits(1) == min(scatter_cdata)), ...
    double(color_limits(2) == max(scatter_cdata)), ...
    double(strcmp(get(axes_handle, 'XGrid'), 'on')), ...
    double(strcmp(get(axes_handle, 'YGrid'), 'on')), ...
    double(strcmp(get(axes_handle, 'ZGrid'), 'on')) ...
];
close(figure_handle);
