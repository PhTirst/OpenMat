index = 0;
total = 0;
while index < 10
    index = index + 1;
    if index == 2
        continue;
    end
    if index == 6
        break;
    end
    total = total + index;
end
openmat_result = [index, total];
