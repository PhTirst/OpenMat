%% 径向波纹曲面 / Radial ripple surface
% surf 直接接受规则网格 X、Y、Z；插值着色显示连续的填充曲面。
% 与 mobius_strip.m 中显式指定顶点和面的 patch 用法对照。

[X, Y] = meshgrid(linspace(-10, 10, 161));
R = sqrt(X.^2 + Y.^2) + eps;
Z = 3*sin(R)./R;

figure('Name', 'Radial Ripple Surface', 'Color', [0.02 0.03 0.05]);
surf(X, Y, Z, 'FaceColor', 'interp', 'EdgeColor', 'none');
axis tight;
axis off;
colormap(turbo);
view(42, 30);
camlight headlight;
lighting gouraud;
title('Ripples | surf', 'Color', 'w', 'FontSize', 18);
