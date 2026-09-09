a = fft([1 2 3 4]);
b = ifft(a);
c = fft2([1 2; 3 4]);
d = fftn(reshape(1:8, [2 2 2]));
openmat_result = [a, b, c(:).', d(:).'];
