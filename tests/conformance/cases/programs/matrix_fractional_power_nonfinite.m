nan_power = [1 NaN; 0 4] ^ 0.5;
inf_power = [1 Inf; 0 4] ^ 0.5;
openmat_result = [sum(isnan(nan_power(:))), sum(isnan(inf_power(:)))];
