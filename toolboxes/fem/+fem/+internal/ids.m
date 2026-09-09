function values = ids(values, count)
%IDS Preserve exact external labels, including zero and sparse uint64 IDs.
    if ~isnumeric(values) || ~isreal(values) || numel(values) ~= count
        error('fem:InvalidId', 'External IDs must have one numeric entry per entity.');
    end
    % Only the sign check uses double; the original labels are never rounded.
    if any(double(values(:)) < 0)
        error('fem:InvalidId', 'External IDs must be nonnegative integers.');
    end
    if ~isinteger(values)
        if any(~isfinite(values(:))) || any(values(:) ~= floor(values(:))) || ...
                any(values(:) > 9007199254740992)
            error('fem:InvalidId', 'Large IDs must be supplied as exact integer arrays.');
        end
    end
    values = uint64(values(:));
end
