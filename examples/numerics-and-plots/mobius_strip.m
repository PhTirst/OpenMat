% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

[u, v] = meshgrid( ...
    linspace(0, 2*pi, 400), ...
    linspace(-0.6, 0.6, 80));

X = (2 + v.*cos(u/2)) .* cos(u);
Y = (2 + v.*cos(u/2)) .* sin(u);
Z = v .* sin(u/2);

C = sin(3*u) + 0.5*cos(8*v);

figure('Color', [0.02 0.02 0.025]);

surf(X, Y, Z, C, ...
    'EdgeColor', 'none', ...
    'FaceColor', 'interp');

axis equal;
axis off;

colormap(turbo);

view(38, 24);

camlight headlight;
lighting gouraud;

title('Mobius Strip', ...
    'Color', 'w', ...
    'FontSize', 18);
