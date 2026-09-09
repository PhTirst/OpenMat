function events = openmat_gc_lifecycle_read(case_id)
global OPENMAT_GC_LIFECYCLE_EVENTS

if isempty(OPENMAT_GC_LIFECYCLE_EVENTS)
    events = zeros(1, 0);
    return;
end
matches = OPENMAT_GC_LIFECYCLE_EVENTS(:, 1) == case_id;
events = reshape(OPENMAT_GC_LIFECYCLE_EVENTS(matches, 2), 1, []);
end
