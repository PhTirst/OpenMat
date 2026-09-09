openmat_result = zeros(1, 3);

first = -2;
if first < 0
    openmat_result(1) = 10;
elseif first == 0
    openmat_result(1) = 11;
else
    openmat_result(1) = 12;
end

second = 0;
if second < 0
    openmat_result(2) = 19;
elseif second == 0
    openmat_result(2) = 20;
else
    openmat_result(2) = 21;
end

third = 2;
if third < 0
    openmat_result(3) = 28;
elseif third == 0
    openmat_result(3) = 29;
else
    openmat_result(3) = 30;
end
