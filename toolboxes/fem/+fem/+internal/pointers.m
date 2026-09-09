function values = pointers(values, count, entries)
%POINTERS Validate a one-based ragged array, including its final sentinel.
    values = fem.internal.indices(values, entries + 1, 'Pointers');
    if numel(values) ~= count + 1 || values(1) ~= 1 || ...
            values(end) ~= entries + 1 || any(diff(values) < 0)
        error('fem:InvalidPointers', 'Invalid one-based ragged-array pointers.');
    end
end
