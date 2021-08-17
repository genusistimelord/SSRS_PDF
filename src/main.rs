use curl::easy::{Auth, Easy};
use lopdf::{Document, Object, ObjectId};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use std::collections::BTreeMap;
use std::fs;

//We create the different ACSII Chaarcters that should be changed to HTML friendly output.
const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

fn main() {
    let mut handle = Easy::new();
    let mut auth = Auth::new();

    //Turn the NTLM Challange on so we can connect.
    auth.ntlm(true);

    //Set the Auth to the Handle so we can use the Username/Password for NTML Authentication
    if let Err(e) = handle.http_auth(&auth) {
        println!("http auth set failed with Error: {}", e);
        return;
    }

    // Set's username for Auth. Generally if Domain is needed then Domain/username otherwise just username
    if let Err(e) = handle.username("usernamehere") {
        println!("username set failed with Error: {}", e);
        return;
    }

    if let Err(e) = handle.password("password") {
        println!("password set failed with Error: {}", e);
        return;
    }

    if let Err(e) = handle.get(true) {
        println!("password set failed with Error: {}", e);
        return;
    }

    /* Url is the Path to the Web service with a ? at the end Then the Folder which the Reports are located in. Then the Report name Without the File ending.
     *  Then Each Parameter Inserted As &Parameter_name=DataSet.
     *  If The parameter Allows multiple Selections then you insert Another of the Same Parameter with a different Dataset.
     *  The DataSet Is a String or Integer. Strings are not formated with "" and are directly placed Spaces in names are fine as long as Web url encoded properly.
     *  &rs:Format tells SSRS what format to return the Report as. In this case we use PDF.
     */
    let url = utf8_percent_encode("http://localhost/ReportServer?/SSRS Folder/ReportName&bottle_count=1&bottle_type=Clear&rs:Format=PDF", FRAGMENT).to_string();
    if let Err(e) = handle.url(&url) {
        println!("url set failed with Error: {}", e);
        return;
    }

    //we build the Byte array starting here which will contain the PDF's Data.
    let mut html: Vec<u8> = Vec::new();

    {
        //We call CURL to send the HTTP request and then get the Data back and set it to the byte array
        let mut transfer = handle.transfer();
        transfer
            .write_function(|data| {
                html.append(&mut Vec::from(data));
                Ok(data.len())
            })
            .unwrap();

        transfer.perform().unwrap();
    };

    //we then write the byte array with a PDF extension as the data is alread a PDF file just in a blob of data
    fs::write("foo.pdf", html).expect("Unable to write file");

    //we can get another PDF from the same request!
    let url = utf8_percent_encode("http://localhost/ReportServer?/SSRS Folder/ReportName&bottle_count=2&bottle_type=Green&rs:Format=PDF", FRAGMENT).to_string();
    if let Err(e) = handle.url(&url) {
        println!("url set failed with Error: {}", e);
        return;
    }

    let mut html: Vec<u8> = Vec::new();

    {
        let mut transfer = handle.transfer();
        transfer
            .write_function(|data| {
                html.append(&mut Vec::from(data));
                Ok(data.len())
            })
            .unwrap();

        transfer.perform().unwrap();
    };

    fs::write("foo2.pdf", html).expect("Unable to write file");

    //we then use lopdf to merge the 2 PDF's with bookmarking.
    merge(&["./foo.pdf".into(), "./foo2.pdf".into()]);
}

pub fn merge(files: &[String]) {
    let mut documents_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut max_id = 1;

    let mut document = Document::with_version("1.5");

    for f in files {
        let mut doc: Document = match Document::load(f) {
            Ok(n) => n,
            Err(e) => {
                println!("{:?}", e);
                return;
            }
        };

        doc.renumber_objects_with(max_id);

        max_id = doc.max_id + 1;

        documents_pages.extend(
            doc.get_pages()
                .into_iter()
                .map(|(_, object_id)| {
                    println!(
                        "Object: {:?} ID: {:?}",
                        doc.get_object(object_id).unwrap(),
                        object_id
                    );
                    (object_id, doc.get_object(object_id).unwrap().to_owned())
                })
                .collect::<BTreeMap<ObjectId, Object>>(),
        );

        documents_objects.extend(doc.objects);
    }

    // Initialize a new empty document

    // Catalog and Pages are mandatory
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    // Process all objects except "Page" type
    for (object_id, object) in documents_objects.iter() {
        // We have to ignore "Page" (as are processed later), "Outlines" and "Outline" objects
        // All other objects should be collected and inserted into the main Document
        match object.type_name().unwrap_or("") {
            "Catalog" => {
                // Collect a first "Catalog" object and use it for the future "Pages"
                catalog_object = Some((
                    if let Some((id, _)) = catalog_object {
                        id
                    } else {
                        *object_id
                    },
                    object.clone(),
                ));
            }
            "Pages" => {
                // Collect and update a first "Pages" object and use it for the future "Catalog"
                // We have also to merge all dictionaries of the old and the new "Pages" object
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, ref object)) = pages_object {
                        if let Ok(old_dictionary) = object.as_dict() {
                            dictionary.extend(old_dictionary);
                        }
                    }

                    pages_object = Some((
                        if let Some((id, _)) = pages_object {
                            id
                        } else {
                            *object_id
                        },
                        Object::Dictionary(dictionary),
                    ));
                }
            }
            "Page" => {}     // Ignored, processed later and separately
            "Outlines" => {} // Ignored, processed later
            "Outline" => {}  // Ignored, processed later
            _ => {
                document.objects.insert(*object_id, object.clone());
            }
        }
    }

    // If no "Pages" found abort
    if pages_object.is_none() {
        println!("Pages root not found.");

        return;
    }

    // Iter over all "Page" and collect with the parent "Pages" created before
    for (object_id, object) in documents_pages.iter() {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", pages_object.as_ref().unwrap().0);

            document
                .objects
                .insert(*object_id, Object::Dictionary(dictionary));
        }
    }

    // If no "Catalog" found abort
    if catalog_object.is_none() {
        println!("Catalog root not found.");

        return;
    }

    let catalog_object = catalog_object.unwrap();
    let pages_object = pages_object.unwrap();

    // Build a new "Pages" with updated fields
    if let Ok(dictionary) = pages_object.1.as_dict() {
        let mut dictionary = dictionary.clone();

        // Set new pages count
        dictionary.set("Count", documents_pages.len() as u32);

        // Set new "Kids" list (collected from documents pages) for "Pages"
        dictionary.set(
            "Kids",
            documents_pages
                .into_iter()
                .map(|(object_id, _)| Object::Reference(object_id))
                .collect::<Vec<_>>(),
        );

        document
            .objects
            .insert(pages_object.0, Object::Dictionary(dictionary));
    }

    // Build a new "Catalog" with updated fields
    if let Ok(dictionary) = catalog_object.1.as_dict() {
        println!("{:?}", dictionary);
        let mut dictionary = dictionary.clone();
        dictionary.set("Pages", pages_object.0);
        dictionary.set("PageMode", "UseOutlines");
        dictionary.remove(b"Outlines"); // Outlines not supported in merged PDFs

        document
            .objects
            .insert(catalog_object.0, Object::Dictionary(dictionary));
    }

    document.trailer.set("Root", catalog_object.0);

    // Update the max internal ID as wasn't updated before due to direct objects insertion
    document.max_id = document.objects.len() as u32;

    // Reorder all new Document objects
    document.renumber_objects();

    //Set any Bookmarks to the First child if they are not set to a page
    document.adjust_zero_pages();

    //Set all bookmarks to the PDF Object tree then set the Outlines to the Bookmark content map.
    if let Some(n) = document.build_outline() {
        if let Ok(Object::Dictionary(ref mut dict)) = document.get_object_mut(catalog_object.0) {
            dict.set("Outlines", Object::Reference(n));
        }
    }

    document.compress();
    // Save the merged PDF
    document.save("merged.pdf").unwrap();
}
