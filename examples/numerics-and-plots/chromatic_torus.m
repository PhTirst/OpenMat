% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

[u, v] = meshgrid( ...
    linspace(0, 2*pi, 300), ...
    linspace(0, 2*pi, 150));

R = 3;
r = 1;

X = (R + r*cos(v)) .* cos(u);
Y = (R + r*cos(v)) .* sin(u);
Z = r * sin(v);

C = sin(3*u) + cos(5*v);

figure('Color', [0.03 0.03 0.04]);

surf(X, Y, Z, C, ...
    'EdgeColor', 'none', ...
    'FaceColor', 'interp');

axis equal;
axis off;

colormap(turbo);

view(42, 28);

camlight headlight;
lighting gouraud;

title('Chromatic Torus', ...
    'Color', 'w', ...
    'FontSize', 18);
