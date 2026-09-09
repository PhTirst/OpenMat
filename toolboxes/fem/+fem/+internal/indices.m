function values = indices(values, upper, name)
%INDICES Validate one-based, exactly representable m-language indices.
    if ~isnumeric(values) || ~isreal(values)
        error('fem:InvalidIndex', '%s must contain numeric indices.', name);
    end
    if isinteger(values) && ~isequal(uint64(double(values)), uint64(values))
        error('fem:InvalidIndex', '%s contains an inexact m-language index.', name);
    end
    values = double(values(:));
    if any(values < 1) || any(values > upper) || ...
            any(values > 9007199254740992)
        error('fem:InvalidIndex', '%s contains an out-of-range index.', name);
    end
    if any(~isfinite(values)) || any(values ~= floor(values))
        error('fem:InvalidIndex', '%s must contain finite integers.', name);
    end
end
