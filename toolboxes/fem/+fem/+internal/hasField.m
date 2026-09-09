function present = hasField(value, name)
%HASFIELD Scalar-struct field probe without requiring a host isfield builtin.
    present = false;
    if ~isstruct(value) || ~isscalar(value)
        return;
    end
    try
        unused = value.(name); %#ok<NASGU> Reading the field is the existence probe.
        present = true;
    catch
        present = false;
    end
end
