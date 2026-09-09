% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

[u, v] = meshgrid( ...
    linspace(0, 6*pi, 500), ...
    linspace(0, 2*pi, 100));

a = 0.18;
b = 0.12;

R = 0.4 + a*u;
r = 0.15 + b*u/(6*pi);

X = R .* cos(u) ...
    + r .* cos(v).*cos(u);

Y = R .* sin(u) ...
    + r .* cos(v).*sin(u);

Z = 0.16*u + r.*sin(v);

C = u + 2*sin(v);

figure('Color', [0.02 0.02 0.025]);

surf(X, Y, Z, C, ...
    'EdgeColor', 'none');

axis equal;
axis off;

colormap(turbo);

view(45, 20);

camlight headlight;
lighting gouraud;

title('Parametric Seashell', ...
    'Color', 'w', ...
    'FontSize', 18);
