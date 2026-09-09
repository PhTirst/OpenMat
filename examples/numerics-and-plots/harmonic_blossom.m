% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

[theta, phi] = meshgrid( ...
    linspace(0, 2*pi, 300), ...
    linspace(0, pi, 180));

r = 1 ...
    + 0.28*sin(6*theta).*sin(phi).^3 ...
    + 0.18*cos(5*phi);

X = r .* sin(phi) .* cos(theta);
Y = r .* sin(phi) .* sin(theta);
Z = r .* cos(phi);

C = sin(6*theta).*sin(phi);

figure('Color', [0.025 0.025 0.035]);

surf(X, Y, Z, C, ...
    'EdgeColor', 'none', ...
    'FaceColor', 'interp');

axis equal;
axis off;

colormap(turbo);

view(40, 25);

camlight headlight;
lighting gouraud;

title('Harmonic Blossom', ...
    'Color', 'w', ...
    'FontSize', 18);
