openmat_result = zeros(1, 3);

if [1, 2]
    openmat_result(1) = 1;
else
    openmat_result(1) = -1;
end

if [1, 0]
    openmat_result(2) = 1;
else
    openmat_result(2) = -1;
end

if zeros(0, 0)
    openmat_result(3) = 1;
else
    openmat_result(3) = -1;
end
