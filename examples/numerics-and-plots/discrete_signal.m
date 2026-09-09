%% 离散信号与量化 / Discrete samples and quantization
% stem 是离散信号常用的茎状图；stairs 显示保持后的阶梯信号。
% 改变 quantizationStep，可观察量化精度的变化。

sampleIndex = 0:40;
samples = exp(-0.055*sampleIndex).*sin(0.22*pi*sampleIndex) ...
    + 0.25*cos(0.60*pi*sampleIndex);
fineTime = linspace(0, 40, 800);
reference = exp(-0.055*fineTime).*sin(0.22*pi*fineTime) ...
    + 0.25*cos(0.60*pi*fineTime);
quantizationStep = 0.25;
quantized = quantizationStep*round(samples/quantizationStep);

figure('Name', 'Discrete Signal', 'Color', [0.98 0.985 1]);
subplot(2, 1, 1);
plot(fineTime, reference, 'Color', [0.77 0.83 0.89], 'LineWidth', 1.2);
hold on;
stem(sampleIndex, samples, 'Color', [0.43 0.35 0.76], ...
    'MarkerFaceColor', [0.61 0.53 0.90], 'MarkerSize', 4, 'LineWidth', 1.3);
set(gca, 'Position', [0.14 0.61 0.80 0.23]);
xlim([0 40]); ylim([-1.2 1.2]);
title('Discrete Samples | stem');
ylabel('Amplitude');
grid on;

subplot(2, 1, 2);
stairs(sampleIndex, quantized, 'Color', [0.06 0.56 0.61], 'LineWidth', 1.8);
hold on;
plot(sampleIndex, samples, '.', 'Color', [0.40 0.42 0.52], 'MarkerSize', 5);
set(gca, 'Position', [0.14 0.16 0.80 0.23]);
xlim([0 40]); ylim([-1.2 1.2]);
title('Quantized, Step = 0.25 | stairs');
xlabel('Sample index n'); ylabel('Amplitude');
grid on;
