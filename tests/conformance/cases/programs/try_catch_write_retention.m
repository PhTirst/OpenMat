retained = 1;
caught_value = 0;
try
    retained = 7;
    value = 1;
    value(2);
    retained = 99;
catch
    caught_value = retained;
end
openmat_result = [retained, caught_value];
