% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

[x, y] = meshgrid(linspace(-5, 5, 240));

r1 = sqrt((x + 1.5).^2 + y.^2);
r2 = sqrt((x - 1.5).^2 + y.^2);

z = sin(14*r1) ./ sqrt(r1 + 0.3) ...
  + sin(14*r2) ./ sqrt(r2 + 0.3);

figure('Color', 'k');

contourf(x, y, z, 48, ...
    'LineColor', 'none');

axis equal tight;
axis off;

colormap(turbo);
colorbar;

title('Wave Interference', ...
    'Color', 'w', ...
    'FontSize', 18);
