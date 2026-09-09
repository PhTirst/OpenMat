function indices = lookupIds(queries, ids)
%LOOKUPIDS Exact integer label remapping, using O(N log N) sort/group work.
% Combined unique returns compact double group IDs without converting labels.
    if isempty(queries)
        indices = zeros(0, 1);
        return;
    end
    count = numel(ids);
    [groups, ~, inverse] = unique([ids(:); queries(:)]);
    indexByGroup = zeros(numel(groups), 1);
    indexByGroup(inverse(1:count)) = (1:count).';
    indices = indexByGroup(inverse(count + 1:end));
    if any(indices == 0)
        error('fem:UnknownId', 'A query references an unknown external ID.');
    end
end
