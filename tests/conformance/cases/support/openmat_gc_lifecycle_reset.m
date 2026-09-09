function openmat_gc_lifecycle_reset(case_id)
global OPENMAT_GC_LIFECYCLE_EVENTS

if isempty(OPENMAT_GC_LIFECYCLE_EVENTS)
    OPENMAT_GC_LIFECYCLE_EVENTS = zeros(0, 2);
    return;
end
OPENMAT_GC_LIFECYCLE_EVENTS( ...
    OPENMAT_GC_LIFECYCLE_EVENTS(:, 1) == case_id, :) = [];
end
