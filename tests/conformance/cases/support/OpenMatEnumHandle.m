classdef OpenMatEnumHandle < handle
    properties
        Value = 0
    end

    methods
        function object = OpenMatEnumHandle(value)
            object.Value = value;
        end
    end

    enumeration
        Only(31)
    end
end
