classdef OpenMatEnumOnce
    properties
        Value = 0
    end

    methods
        function object = OpenMatEnumOnce(value)
            global openmat_enum_once_count
            openmat_enum_once_count = openmat_enum_once_count + 1;
            object.Value = value;
        end
    end

    enumeration
        First(17)
    end
end
