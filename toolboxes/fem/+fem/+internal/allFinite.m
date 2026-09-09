function valid = allFinite(value)
%ALLFINITE Check sparse numeric storage without densifying implicit zeros.
    if issparse(value)
        entries = nonzeros(value);
    else
        entries = value(:);
    end
    valid = all(isfinite(entries));
end
