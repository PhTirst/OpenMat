%% 莫比乌斯面片 / Mobius strip built with patch
% 显式构造 Vertices 顶点表和 Faces 四边形面表，再交给 patch 绘制。
% 最后一圈反向连接第一圈，闭合半扭转的带面；网格边缘可单独设色。

ringSegments = 96;
widthSamples = 15;
[u, v] = meshgrid( ...
    (0:ringSegments-1)*(2*pi/ringSegments), ...
    linspace(-0.6, 0.6, widthSamples));

X = (2 + v.*cos(u/2)) .* cos(u);
Y = (2 + v.*cos(u/2)) .* sin(u);
Z = v .* sin(u/2);

C = cos(2*u) + 0.4*cos(5*v);
vertices = [X(:), Y(:), Z(:)];
faces = zeros(ringSegments*(widthSamples-1), 4);
for ring = 1:ringSegments
    for band = 1:widthSamples-1
        faceIndex = (ring-1)*(widthSamples-1) + band;
        first = (ring-1)*widthSamples + band;
        if ring < ringSegments
            next = ring*widthSamples + band;
            faces(faceIndex, :) = [first first+1 next+1 next];
        else
            faces(faceIndex, :) = [first first+1 widthSamples-band widthSamples-band+1];
        end
    end
end

figure('Name', 'Mobius Patch', 'Color', [0.02 0.03 0.05]);

patch('Vertices', vertices, 'Faces', faces, 'FaceVertexCData', C(:), ...
    'FaceColor', 'interp', ...
    'EdgeColor', [0.10 0.14 0.20], 'LineWidth', 0.6);
hold on;

% 沿 0 到 4*pi 绕行一次，描出莫比乌斯带唯一的边界曲线。
boundary = linspace(0, 4*pi, 700);
bx = (2 + 0.6*cos(boundary/2)) .* cos(boundary);
by = (2 + 0.6*cos(boundary/2)) .* sin(boundary);
bz = 0.6*sin(boundary/2);
plot3(bx, by, bz, 'Color', [0.78 0.94 1], 'LineWidth', 1.4);

axis equal;
axis off;

colormap(turbo);

view(38, 24);

camlight headlight;
lighting gouraud;

title('Mobius | patch', ...
    'Color', 'w', ...
    'FontSize', 18);
