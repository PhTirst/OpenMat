function [rows, inverse] = uniquePairs(pairs)
%UNIQUEPAIRS Lexicographic pair deduplication using vector sort operations.
% Avoid packed floating-point keys and host-specific unique(...,'rows').
    count = size(pairs, 1);
    inverse = zeros(count, 1);
    if count == 0
        rows = zeros(0, 2);
        return;
    end
    [first, permutation] = sort(pairs(:, 1));
    start = 1;
    while start <= count
        stop = start;
        while stop < count && first(stop + 1) == first(start)
            stop = stop + 1;
        end
        slots = start:stop;
        group = permutation(slots);
        [~, order] = sort(pairs(group, 2));
        permutation(slots) = group(order);
        start = stop + 1;
    end
    sorted = pairs(permutation, :);
    fresh = [true; any(diff(sorted, 1, 1) ~= 0, 2)];
    rows = sorted(fresh, :);
    inverse(permutation) = cumsum(double(fresh));
end
