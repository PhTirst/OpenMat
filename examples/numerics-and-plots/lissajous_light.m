% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

t = linspace(0, 2*pi, 5000);

x = sin(5*t + pi/2);
y = sin(4*t);

figure('Color', [0.02 0.02 0.03]);
hold on;

scatter(x, y, 8, t, 'filled');

axis equal;
axis off;

colormap(turbo);

title('Lissajous Light', ...
    'Color', 'w', ...
    'FontSize', 18);
