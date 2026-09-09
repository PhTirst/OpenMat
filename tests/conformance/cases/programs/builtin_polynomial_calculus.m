coefficients = poly([1 2 3]);
values = polyval([1 2 3], [0 1; 2 3]);
derivative = polyder([1 2 3]);
[ratio_numerator, ratio_denominator] = polyder([1 2], [3 4]);
integral = polyint([1 2 3], 4);
openmat_result = [ ...
    coefficients, values(:).', derivative, ratio_numerator, ratio_denominator, integral ...
];
