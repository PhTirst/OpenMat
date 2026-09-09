object = OpenMatAccessDerived;
object.PublicValue = 10;
object = object.setProtected(20);
openmat_result = [object.PublicValue, object.readProtected(), ...
    object.readPrivate()];
