classdef OpenMatDependentBox
    properties (Access = private)
        Base
    end

    properties (Dependent)
        Twice
    end

    methods
        function object = OpenMatDependentBox(value)
            object.Base = value;
        end

        function value = get.Twice(object)
            value = object.Base * 2;
        end

        function object = set.Twice(object, value)
            object.Base = value / 2;
        end
    end
end
