a = fftshift(1:5);
b = ifftshift(1:5);
c = ifft([1 2 3], 'symmetric');
e = fft(zeros(0, 3), 2);
openmat_result = [a, b, c, size(e), double(isempty(fft([]))), double(isreal(c))];
