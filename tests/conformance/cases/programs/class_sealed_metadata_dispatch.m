object = OpenMatSealedBase();
metadata = metaclass(object);
openmat_result = [object.calculate(3), metadata.Abstract, metadata.Sealed];
