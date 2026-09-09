%% FFT：从混合信号中找出频率 / Find the frequencies in a signal
% 打开本文件并点击 Run。改变 f1、f2 或幅度，观察频谱峰值。
% 使用整数周期，避免频谱泄漏；不需要信号处理工具箱。

fs = 1000;
N = 1000;
t = (0:N-1) / fs;
f1 = 50;
f2 = 120;
signal = 0.8*sin(2*pi*f1*t) + 0.35*cos(2*pi*f2*t);

transform = fft(signal);
amplitude = abs(transform(1:N/2+1)) / N;
amplitude(2:end-1) = 2 * amplitude(2:end-1);
frequency = (0:N/2) * fs / N;

figure('Name', 'FFT Spectrum', 'Color', [0.98 0.98 1]);
subplot(2, 1, 1);
plot(t, signal, 'Color', [0.13 0.55 0.65], 'LineWidth', 1.8);
set(gca, 'Position', [0.14 0.61 0.8 0.23]);
xlim([0 0.12]);
xlabel('Time (s)');
ylabel('Amplitude');
title('Signal in time');
grid on;

subplot(2, 1, 2);
plot(frequency, amplitude, 'Color', [0.48 0.35 0.78], 'LineWidth', 2);
set(gca, 'Position', [0.14 0.16 0.8 0.23]);
xlim([0 200]);
ylim([0 1]);
xlabel('Frequency (Hz)');
ylabel('Single-sided amplitude');
title('Peaks at 50 Hz and 120 Hz');
grid on;

disp('FFT complete: the peaks are 50 Hz (0.8) and 120 Hz (0.35).');
