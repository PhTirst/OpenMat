% 打开本文件并点击 Run。可修改下方采样量、参数或颜色。
% Open this script and click Run. No extra data or toolbox is required.

[theta, phi] = meshgrid( ...
    linspace(0, 2*pi, 360), ...
    linspace(0, pi, 220));

r = 1 ...
    + 0.10*sin(10*theta).*sin(8*phi) ...
    + 0.06*cos(18*phi);

X = r .* sin(phi).*cos(theta);
Y = r .* sin(phi).*sin(theta);
Z = r .* cos(phi);

C = sin(8*phi) + cos(10*theta);

figure('Color', [0.015 0.015 0.025]);

surf(X,Y,Z,C, ...
    'EdgeColor','none');

axis equal;
axis off;

colormap(turbo);

view(35,25);

camlight headlight;
lighting gouraud;

title('Wave Planet', ...
    'Color','w', ...
    'FontSize',18);
