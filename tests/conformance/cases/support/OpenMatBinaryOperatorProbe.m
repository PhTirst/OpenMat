classdef OpenMatBinaryOperatorProbe
    properties
        Value
    end

    methods
        function object = OpenMatBinaryOperatorProbe(value)
            object.Value = value;
        end

        function result = plus(left, right)
            result = 1;
        end

        function result = minus(left, right)
            result = 2;
        end

        function result = mtimes(left, right)
            result = 3;
        end

        function result = times(left, right)
            result = 4;
        end

        function result = mrdivide(left, right)
            result = 5;
        end

        function result = mldivide(left, right)
            result = 6;
        end

        function result = rdivide(left, right)
            result = 7;
        end

        function result = ldivide(left, right)
            result = 8;
        end

        function result = mpower(left, right)
            result = 9;
        end

        function result = power(left, right)
            result = 10;
        end

        function result = eq(left, right)
            result = true;
        end

        function result = ne(left, right)
            result = false;
        end

        function result = lt(left, right)
            result = true;
        end

        function result = le(left, right)
            result = false;
        end

        function result = gt(left, right)
            result = true;
        end

        function result = ge(left, right)
            result = false;
        end
    end
end
