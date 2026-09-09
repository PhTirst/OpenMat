classdef (Sealed) OpenMatAbstractLeaf < OpenMatAbstractMiddle
    methods
        function value = offset(object, input)
            value = input * 3;
        end
    end

    methods (Access = protected)
        function value = hidden(object, input)
            value = input * 4;
        end
    end
end
