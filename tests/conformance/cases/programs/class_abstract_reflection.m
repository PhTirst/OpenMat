object = OpenMatAbstractLeaf();
base_metadata = meta.class.fromName('OpenMatAbstractBase');
leaf_metadata = metaclass(object);
method_list = base_metadata.MethodList;
abstract_count = 0;
for index = 1:numel(method_list)
    abstract_count = abstract_count + method_list(index).Abstract;
end
openmat_result = [base_metadata.Abstract, base_metadata.Sealed, ...
    leaf_metadata.Abstract, leaf_metadata.Sealed, abstract_count];
