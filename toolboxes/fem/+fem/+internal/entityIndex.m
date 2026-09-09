function value = entityIndex(value, upper, name)
%ENTITYINDEX Validate queries that refer to exactly one entity.
    value = fem.internal.indices(value, upper, name);
    if numel(value) ~= 1
        error('fem:InvalidIndex', '%s must select exactly one entity.', name);
    end
end
